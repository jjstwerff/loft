// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//
// @PLN98 P3.4 — drive the INTERACTIVE browser debug client of a
// `loft --html --debug` wasm: queue a `bp -> run -> eval -> resume` sequence of
// `D!:` control frames as the JS->wasm `host_input`, call `loft_debug_start` +
// `loft_debug_pump`, and print the wasm's `D:` replies. This is what the server
// relay does over the WebSocket the client holds.
import fs from 'node:fs';
const wasm = fs.readFileSync(process.argv[2]);
const enc = new TextEncoder(), dec = new TextDecoder();
let mem = null, out = '';
const inQ = ['D!:bp compute', 'D!:run', 'D!:eval n', 'D!:eval n + 2', 'D!:resume'].map(s => enc.encode(s));
const io = {
  loft_host_print: (p, l) => { out += dec.decode(new Uint8Array(mem.buffer, p, l)); },
  loft_host_input_len: () => (inQ.length ? inQ[0].length : 0),
  loft_host_input_copy: (p) => { const b = inQ.shift(); if (b) new Uint8Array(mem.buffer, p, b.length).set(b); },
  loft_host_output: () => {},
};
// loft_io gets a per-function callable fallback too, so a newly-added import
// (e.g. loft_host_http_get) never LinkErrors a stub harness that never calls it.
const stubs = new Proxy({ loft_io: new Proxy(io, { get: (t, k) => (k in t ? t[k] : () => 0) }) }, { get: (t, k) => (k in t ? t[k] : new Proxy({}, { get: () => () => 0 })) });
const inst = new WebAssembly.Instance(new WebAssembly.Module(wasm), stubs);
mem = inst.exports.memory;
const started = inst.exports.loft_debug_start();
inst.exports.loft_debug_pump();
process.stdout.write(out);
console.log('STARTED=' + started);
