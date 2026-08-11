// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//
// @PLN98 P3.4 — drive `loft_debug_selftest` in a `loft --html --debug` wasm to
// confirm the interpreter's cooperative debug cycle (breakpoint pause + frame
// read + resume) runs on wasm32.  Prints the wasm's host output, then a final
// `RETURN=<0|1>` line (1 = the cycle produced the expected outcome).
import fs from 'node:fs';
// loft#851 — the page's filesystem, imported rather than restubbed so every
// harness answers what a real page answers.  A stub returning 0 would mean "an
// empty file that EXISTS" where the contract says absent, and a missing import
// is a LinkError the moment a program under test touches a file.
import { loftFSImports } from '../doc/loft-fs.js';
const wasm = fs.readFileSync(process.argv[2]);
const dec = new TextDecoder();
let mem = null;
let out = '';
const io = {
  ...loftFSImports(() => mem),
  loft_host_print: (p, l) => { out += dec.decode(new Uint8Array(mem.buffer, p, l)); },
  loft_host_input_len: () => 0,
  loft_host_input_copy: () => {},
  loft_host_output: () => {},
};
// Answer any other import module/name with a 0-returning stub (as wasm_repro.mjs).
// loft_io gets a per-function callable fallback too, so a newly-added import
// (e.g. loft_host_http_get) never LinkErrors a stub harness that never calls it.
const stubs = new Proxy({ loft_io: new Proxy(io, { get: (t, k) => (k in t ? t[k] : () => 0) }) }, { get: (t, k) => (k in t ? t[k] : new Proxy({}, { get: () => () => 0 })) });
const inst = new WebAssembly.Instance(new WebAssembly.Module(wasm), stubs);
mem = inst.exports.memory;
const rc = inst.exports.loft_debug_selftest();
process.stdout.write(out);
console.log('RETURN=' + rc);
