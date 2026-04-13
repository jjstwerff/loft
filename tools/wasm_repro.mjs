// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//
// P137 regression harness: instantiate a loft `--html`-built WASM in
// Node with stub host imports, run `loft_start`, and report whether
// the module traps.
//
// Usage (release wasm):
//   node tools/wasm_repro.mjs <path/to/wasm_file> [--trace]
//
// Extracting the WASM from a `--html` HTML bundle:
//   python3 -c "import re,base64,sys;html=open(sys.argv[1]).read();\
//     m=re.search(r'wasmB64=\"([A-Za-z0-9+/=]+)\"',html);\
//     open(sys.argv[2],'wb').write(base64.b64decode(m.group(1)))" \
//     doc/brick-buster.html /tmp/bb.wasm
//
// Exit code 0 on clean run, 1 on trap.  The --trace flag enables a
// global buffer that records every host import call, printed on trap
// to narrow down which loft function last entered the host boundary
// before the fault.
//
// This harness found the P137 root cause: a `fn main() {}` HTML trap
// produced an empty trace (no host import reached), proving the bug
// was in WASM init, not in any host-call marshalling.

import fs from 'node:fs';
import process from 'node:process';

const argv = process.argv.slice(2);
if (argv.length < 1) {
  console.error('usage: node wasm_repro.mjs <wasm_file> [--trace]');
  process.exit(2);
}

const wasmPath = argv[0];
const enableTrace = argv.includes('--trace');
const wasm = fs.readFileSync(wasmPath);
const trace = [];
let instance = null;

// Expected host imports for a `loft --html` bundle.  Loose stubs —
// we don't care what they return; we only care whether a trap fires
// during loft_start.
const stubs = {
  loft_io: {
    loft_host_print: (ptr, len) => {
      if (enableTrace) trace.push(`loft_host_print(${ptr}, ${len})`);
      if (instance) {
        const mem = instance.exports.memory;
        const bytes = new Uint8Array(mem.buffer, ptr, len);
        const s = new TextDecoder().decode(bytes);
        if (!enableTrace) process.stdout.write(s);
      }
    },
  },
  loft_gl: new Proxy({}, {
    // Any loft_gl.* call becomes a no-op stub; record the name when
    // tracing is on.  The proxy lets us answer for every imported
    // function name without having to enumerate them.
    get(_target, name) {
      return (...args) => {
        if (enableTrace) trace.push(`loft_gl.${String(name)}(${args.length} args)`);
        return 0;
      };
    },
  }),
};

const mod = new WebAssembly.Module(wasm);
try {
  instance = new WebAssembly.Instance(mod, stubs);
} catch (e) {
  console.error(`instantiate failed: ${e.message}`);
  process.exit(1);
}

console.error(`exports: ${Object.keys(instance.exports).join(',')}`);

try {
  instance.exports.loft_start();
  console.error('loft_start: OK');
  process.exit(0);
} catch (e) {
  console.error(`TRAP: ${e.message}`);
  if (enableTrace) {
    console.error('last trace entries:');
    for (const entry of trace.slice(-20)) console.error(`  ${entry}`);
  }
  console.error('stack:');
  console.error(e.stack);
  process.exit(1);
}
