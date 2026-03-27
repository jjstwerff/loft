// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

/**
 * W1.12 — Build script: generate ide/assets/base-fs.json
 *
 * Reads:
 *   tests/docs/*.loft   → tree["/"].examples
 *   doc/*.html          → tree["/"].docs
 *   default/*.loft      → tree["/"].lib
 *
 * Writes:
 *   ide/assets/base-fs.json
 *
 * Run from the repository root:
 *   node ide/scripts/build-base-fs.js
 *
 * The output is loaded once on Web IDE startup and never written to.
 * The VirtFS delta (user edits) is stored separately in localStorage.
 */

import { readFileSync, readdirSync, writeFileSync, existsSync } from 'fs';
import { join } from 'path';

// ── Source directories ─────────────────────────────────────────────────────────

const SOURCES = [
  { dir: 'tests/docs', section: 'examples', ext: '.loft' },
  { dir: 'doc',        section: 'docs',     ext: '.html'  },
  { dir: 'default',    section: 'lib',      ext: '.loft'  },
];

const OUT = 'ide/assets/base-fs.json';

// ── Build ──────────────────────────────────────────────────────────────────────

const tree = { '/': { examples: {}, docs: {}, lib: {} } };
let totalFiles = 0;

for (const { dir, section, ext } of SOURCES) {
  if (!existsSync(dir)) {
    console.warn(`WARN  directory not found, skipping: ${dir}`);
    continue;
  }

  const files = readdirSync(dir)
    .filter(f => f.endsWith(ext))
    .sort();   // stable order across platforms

  for (const f of files) {
    const content = readFileSync(join(dir, f), 'utf8');
    tree['/'][section][f] = { $type: 'text', $content: content };
    totalFiles++;
  }

  console.log(`  ${section.padEnd(8)} ${files.length} file(s) from ${dir}/`);
}

const json = JSON.stringify(tree);
writeFileSync(OUT, json);

const kb = (json.length / 1024).toFixed(1);
console.log(`\nWrote ${OUT}  (${totalFiles} files, ${kb} KB uncompressed)`);
