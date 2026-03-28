// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

/**
 * W1.13 — Single-test WASM runner (subprocess helper).
 *
 * Called by suite.mjs with:
 *   node tests/wasm/run-one.mjs <filePath>
 *
 * Loads a fresh WASM module, runs the given .loft file, and writes the
 * JSON result to stdout.  Running each test in its own subprocess guarantees
 * that a WASM crash (RuntimeError: unreachable / memory access out of bounds)
 * does not corrupt the module state and cause all subsequent tests to fail.
 */

import { readFileSync } from 'fs';
import { basename } from 'path';
import { createHost } from './host.mjs';
import { buildDefaultTree, withFiles } from './default-tree.mjs';

const [, , filePath] = process.argv;
if (!filePath) {
  process.stderr.write('Usage: node run-one.mjs <filePath>\n');
  process.exit(2);
}

let compileAndRun;
try {
  ({ compile_and_run: compileAndRun } = await import('./pkg/loft.js'));
} catch {
  process.stdout.write(JSON.stringify({ success: false, output: '', diagnostics: 'WASM package not built' }));
  process.exit(0);
}

const name = basename(filePath);
let content;
try {
  content = readFileSync(filePath, 'utf8');
} catch (err) {
  process.stdout.write(JSON.stringify({ success: false, output: '', diagnostics: String(err) }));
  process.exit(0);
}

const _defaultTree = buildDefaultTree();
const tree = JSON.parse(JSON.stringify(_defaultTree));
tree['/'].tmp = {};
const finalTree = withFiles(tree, { [filePath]: content });

const { host } = createHost(finalTree);
globalThis.loftHost = host;

try {
  const raw = compileAndRun(JSON.stringify([{ name, content }]));
  process.stdout.write(raw);
} catch (err) {
  process.stdout.write(JSON.stringify({
    success: false,
    output: '',
    diagnostics: String(err),
  }));
}
