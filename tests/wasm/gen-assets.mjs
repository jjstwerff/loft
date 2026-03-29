#!/usr/bin/env node
// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

/**
 * W1.16 — Generate tests/wasm/assets.json from the real filesystem.
 *
 * Reads tests/docs/, tests/scripts/, and tests/example/ into a VirtFS-
 * compatible JSON tree and writes it to tests/wasm/assets.json.
 *
 * The generated file can be imported directly in browser contexts where
 * Node.js `fs` APIs are unavailable:
 *
 *   import assets from './assets.json' with { type: 'json' };
 *   const { host } = createHost(assets);
 *
 * Usage:
 *   node tests/wasm/gen-assets.mjs [--root <project-root>] [--out <output-file>]
 *
 * Run this whenever tests/docs/, tests/scripts/, or tests/example/ change.
 * In CI, run it before the WASM test suite to keep assets.json up to date.
 * Add a `make wasm-assets` target to make this discoverable.
 */

'use strict';

import { writeFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { buildDefaultTree } from './default-tree.mjs';

// ── CLI argument parsing ───────────────────────────────────────────────────────

const args = process.argv.slice(2);
let projectRoot = '.';
let outFile     = 'tests/wasm/assets.json';

for (let i = 0; i < args.length; i++) {
  if (args[i] === '--root' && args[i + 1]) { projectRoot = args[++i]; }
  if (args[i] === '--out'  && args[i + 1]) { outFile     = args[++i]; }
}

// ── Build and write ────────────────────────────────────────────────────────────

const absRoot = resolve(projectRoot);
const absOut  = resolve(outFile);

console.log(`Building default VirtFS tree from ${absRoot} …`);
const tree = buildDefaultTree({ root: absRoot });

// Count entries for feedback
let files = 0, dirs = 0;
function countNodes(node) {
  for (const [, v] of Object.entries(node)) {
    if (v.$type) { files++; } else { dirs++; countNodes(v); }
  }
}
countNodes(tree['/']);

const json = JSON.stringify(tree, null, 2);
writeFileSync(absOut, json, 'utf8');

console.log(`Written ${absOut}`);
console.log(`  ${files} files, ${dirs} directories, ${(json.length / 1024).toFixed(1)} KB`);
