// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

/**
 * W1.16 — Default VirtFS tree for WASM tests and browser demos.
 *
 * Exports {@link buildDefaultTree} which returns a VirtFS-compatible JSON tree
 * pre-populated with the loft documentation examples and script fixtures.
 * The tree matches the real on-disk layout so that loft programs can reference
 * paths like `tests/example/config/terrain.txt` without modification.
 *
 * In Node.js the files are read from disk on demand.
 * In a browser context, import the pre-generated `assets.json` instead and
 * pass it directly to {@link https://github.com/jurjen/loft|createHost}:
 *
 *   import assets from './assets.json' with { type: 'json' };
 *   const { host } = createHost(assets);
 *
 * To regenerate `assets.json` after changing any source file:
 *   node tests/wasm/gen-assets.mjs
 *
 * VirtFS node format:
 *   - Directory: plain object (no `$type` key)
 *   - Text file: { $type: 'text', $content: '<utf-8 string>' }
 *   - Binary file: { $type: 'binary', $content: '<base64 string>' }
 */

'use strict';

// ── Node.js fs import (absent in browser) ─────────────────────────────────────

let _fs = null;
let _path = null;
try {
  _fs   = await import('node:fs');
  _path = await import('node:path');
} catch {
  // Running in a browser — caller must supply a pre-generated tree.
}

// ── Directories included in the default tree ──────────────────────────────────

/**
 * Directories (relative to the project root) that are included in the default
 * VirtFS tree.  The order determines the nesting in the final tree object.
 */
export const DEFAULT_DIRS = [
  'tests/docs',
  'tests/scripts',
  'tests/example',
];

/**
 * Individual files (relative to the project root) that are always included,
 * even if their parent directory is not in DEFAULT_DIRS.
 */
export const DEFAULT_FILES = [
  // nothing extra — all needed files live under DEFAULT_DIRS
];

// ── Helpers ───────────────────────────────────────────────────────────────────

/** Encode a Uint8Array (or Buffer) to a base64 string. */
function _b64(bytes) {
  if (typeof Buffer !== 'undefined') return Buffer.from(bytes).toString('base64');
  let s = '';
  for (let i = 0; i < bytes.length; i++) s += String.fromCharCode(bytes[i]);
  return btoa(s);
}

/**
 * Return true if the Buffer contains only valid UTF-8 text with no null bytes.
 * A quick heuristic: re-encode as UTF-8 and check the byte length matches.
 */
function _isUtf8(buf) {
  // Null bytes are a reliable binary indicator.
  if (buf.includes(0)) return false;
  // Re-encode the decoded string and check round-trip length.
  try {
    const str = buf.toString('utf8');
    return Buffer.byteLength(str, 'utf8') === buf.length;
  } catch {
    return false;
  }
}

/**
 * Attempt to read `filePath` as UTF-8 text; fall back to base64 binary.
 * Returns `{ $type: 'text'|'binary', $content: string }`, or `null` on error.
 */
function _readNode(filePath) {
  if (!_fs) return null;
  try {
    const bytes = _fs.readFileSync(filePath);
    if (_isUtf8(bytes)) {
      return { $type: 'text', $content: bytes.toString('utf8') };
    }
    return { $type: 'binary', $content: _b64(bytes) };
  } catch {
    return null;
  }
}

/**
 * Recursively populate `node` (a VirtFS directory object) from `diskPath`.
 * Entries whose names start with `.` are skipped (generated/hidden dirs).
 *
 * @param {string} diskPath  Path on the real filesystem.
 * @param {object} node      VirtFS directory node to fill.
 */
function _populateDir(diskPath, node) {
  if (!_fs || !_fs.existsSync(diskPath)) return;
  for (const entry of _fs.readdirSync(diskPath)) {
    if (entry.startsWith('.')) continue;  // skip hidden / generated dirs
    const full = _path.join(diskPath, entry);
    const st = _fs.statSync(full);
    if (st.isDirectory()) {
      node[entry] = {};
      _populateDir(full, node[entry]);
    } else if (st.isFile()) {
      const fileNode = _readNode(full);
      if (fileNode) node[entry] = fileNode;
    }
  }
}

/**
 * Navigate (and create) the VirtFS directory node for `segments` inside `root`.
 * Returns the leaf directory object.
 *
 * @param {object}   root      The root VirtFS object (i.e. `tree['/']`).
 * @param {string[]} segments  Path components, e.g. `['tests', 'example']`.
 * @returns {object}
 */
function _ensureDir(root, segments) {
  let node = root;
  for (const seg of segments) {
    if (!node[seg]) node[seg] = {};
    node = node[seg];
  }
  return node;
}

// ── Public API ────────────────────────────────────────────────────────────────

/**
 * Build the default VirtFS tree from the real filesystem (Node.js only).
 *
 * The returned object can be passed directly to `createHost(tree)`:
 *
 * ```js
 * import { buildDefaultTree } from './default-tree.mjs';
 * import { createHost } from './host.mjs';
 *
 * const { host } = createHost(buildDefaultTree());
 * globalThis.loftHost = host;
 * ```
 *
 * @param {object}  [options]
 * @param {string}  [options.root='.']  Project root directory (cwd by default).
 * @param {string[]} [options.dirs]     Override the directories to include.
 * @returns {object}  VirtFS tree ({ '/': { ... } }).
 */
export function buildDefaultTree(options = {}) {
  const root    = options.root ?? '.';
  const dirs    = options.dirs ?? DEFAULT_DIRS;

  const tree = { '/': {} };

  for (const rel of dirs) {
    const diskPath = _path ? _path.join(root, rel) : rel;
    const segments = rel.split('/').filter(Boolean);
    const node     = _ensureDir(tree['/'], segments);
    _populateDir(diskPath, node);
  }

  for (const rel of DEFAULT_FILES) {
    const diskPath = _path ? _path.join(root, rel) : rel;
    const segments = rel.split('/').filter(Boolean);
    // Parent directory
    const parentNode = _ensureDir(tree['/'], segments.slice(0, -1));
    const name       = segments[segments.length - 1];
    const fileNode   = _readNode(diskPath);
    if (fileNode) parentNode[name] = fileNode;
  }

  return tree;
}

/**
 * Merge `extra` entries into a previously built tree.
 * Useful for adding a user source file to the default tree:
 *
 * ```js
 * const tree = buildDefaultTree();
 * withFiles(tree, { 'project/main.loft': 'fn main() { println("hi") }' });
 * ```
 *
 * @param {object} tree   VirtFS tree to modify in place.
 * @param {object} files  Map of VirtFS paths (slash-separated) to string content.
 * @returns {object}      The modified tree (same reference).
 */
export function withFiles(tree, files) {
  for (const [virtPath, content] of Object.entries(files)) {
    const segments = virtPath.split('/').filter(Boolean);
    const parentNode = _ensureDir(tree['/'], segments.slice(0, -1));
    const name       = segments[segments.length - 1];
    parentNode[name] = { $type: 'text', $content: content };
  }
  return tree;
}
