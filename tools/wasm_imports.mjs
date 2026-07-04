#!/usr/bin/env node
// tools/wasm_imports.mjs — check whether a `loft --html` bundle's embedded WASM
// module imports a given host symbol.
//
// Usage:   node tools/wasm_imports.mjs <bundle.html> <import-name>
// Exit 0 : the module imports <import-name>.
// Exit 1 : it does not (the symbol was dropped/stubbed by the codegen).
// Exit 2 : usage / parse error.
//
// This runs the WASM "natively" (via the JS WebAssembly engine) — no browser —
// so a CI job can gate the `--html` GL codegen without SwiftShader.  It backs
// tests/html_gl_imports.rs, guarding the D-html-vec regression where every
// vector-argument host import (gl_upload_vertices / gl_upload_canvas /
// gl_set_mat4) was silently elided → a blank WebGL canvas.
import fs from 'node:fs';

const [, , htmlPath, needle] = process.argv;
if (!htmlPath || !needle) {
  console.error('usage: wasm_imports.mjs <bundle.html> <import-name>');
  process.exit(2);
}

let html;
try {
  html = fs.readFileSync(htmlPath, 'utf8');
} catch (e) {
  console.error('cannot read ' + htmlPath + ': ' + e.message);
  process.exit(2);
}

// The `--html` bundle embeds the module as base64 beginning with the WASM magic
// `\0asm` → "AGFzbQ".
const m = html.match(/AGFzbQ[A-Za-z0-9+/]+=*/);
if (!m) {
  console.error('no embedded WASM base64 found in ' + htmlPath);
  process.exit(2);
}

let imports;
try {
  const mod = new WebAssembly.Module(Buffer.from(m[0], 'base64'));
  imports = WebAssembly.Module.imports(mod);
} catch (e) {
  console.error('WASM compile failed: ' + e.message);
  process.exit(2);
}

const names = imports.map((i) => i.name);
if (names.includes(needle)) {
  console.log('OK: ' + needle + ' is imported');
  process.exit(0);
}
console.error(
  'MISSING: ' + needle + ' is NOT imported. Present GL-ish imports: ' +
    names.filter((n) => /gl_|upload|vert|canvas|mat/i.test(n)).join(', '),
);
process.exit(1);
