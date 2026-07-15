// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//
// @PLN105 Phase 2d — the deliver node harness (the whole-slice falsifier for Phase 2).
// Instantiates a `loft --html`-built WASM with loft_io host stubs, wires `loft_host_deliver` to the
// generic descriptor-driven reader (doc/loft-deliver.js), runs `loft_start`, and prints every
// delivered value as JSON on stdout. The Rust test (tests/deliver_wasm.rs) asserts that JSON equals
// the value the loft program delivered — proving JS reconstructs a value from wasm memory driven
// only by the descriptor, and (via the same program on --interpret) that --html == interpreter.
//
//   node tools/deliver_repro.mjs <path/to/wasm_file>
//
// Exit 0 on a clean run (deliveries printed as `DELIVER <json>` lines), 1 on a trap/error.

import fs from "node:fs";
import process from "node:process";
import { readLoftValue } from "../doc/loft-deliver.js";

const wasmPath = process.argv[2];
if (!wasmPath) {
  console.error("usage: node deliver_repro.mjs <wasm_file>");
  process.exit(2);
}

let mem = null;
const dec = new TextDecoder();

// JSON-safe: BigInt → Number when exact, else a "…n" string; typed arrays → plain arrays.
function jsonSafe(v) {
  if (typeof v === "bigint")
    return v >= BigInt(Number.MIN_SAFE_INTEGER) && v <= BigInt(Number.MAX_SAFE_INTEGER)
      ? Number(v)
      : `${v}n`;
  if (ArrayBuffer.isView(v)) return Array.from(v, jsonSafe);
  if (Array.isArray(v)) return v.map(jsonSafe);
  if (v && typeof v === "object") {
    const o = {};
    for (const k of Object.keys(v)) o[k] = jsonSafe(v[k]);
    return o;
  }
  return v;
}

const loft_io = {
  loft_host_print(ptr, len) {
    process.stderr.write(dec.decode(new Uint8Array(mem.buffer, ptr, len)));
  },
  loft_host_input_len: () => 0,
  loft_host_input_copy: () => {},
  loft_host_output(ptr, len) {
    process.stderr.write("[out] " + dec.decode(new Uint8Array(mem.buffer, ptr, len)));
  },
  loft_host_http_get: () => 0xffffffff,
  loft_host_http_get_copy: () => {},
  // @PLN105 Phase 2 — the value is handed over as (store_base, rec, pos, type_id) + the descriptor
  // JSON blob (dptr,dlen). Read it synchronously here (the borrow ends when this returns), copying
  // out of the zero-copy typed-array views so the captured value survives past the call.
  loft_host_deliver(tag, store_base, rec, pos, type_id, dptr, dlen) {
    const desc = JSON.parse(dec.decode(new Uint8Array(mem.buffer, dptr, dlen)));
    const value = readLoftValue(mem, store_base, desc, type_id, rec, pos);
    const line = { tag: typeof tag === "bigint" ? Number(tag) : tag, type: type_id, value: jsonSafe(value) };
    process.stdout.write("DELIVER " + JSON.stringify(line) + "\n");
  },
};

try {
  const bytes = fs.readFileSync(wasmPath);
  const { instance } = await WebAssembly.instantiate(bytes, { loft_io });
  mem = instance.exports.memory;
  if (typeof instance.exports.loft_start === "function") instance.exports.loft_start();
  else if (typeof instance.exports.main === "function") instance.exports.main();
  process.exit(0);
} catch (e) {
  console.error("deliver_repro: " + (e && e.stack ? e.stack : e));
  process.exit(1);
}
