// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

/**
 * W1.10 — In-memory virtual filesystem for WASM tests.
 *
 * The filesystem is represented as a JSON tree:
 *   - A key whose value is a plain `{}` or contains nested keys (without `$type`)
 *     is a **directory**.
 *   - `{ "$type": "text", "$content": "..." }` is a **text file**.
 *   - `{ "$type": "binary", "$content": "<base64>" }` is a **binary file**.
 *
 * Special keys always start with `$`.  No loft filename may start with `$`.
 */

'use strict';

// ── Base64 helpers ─────────────────────────────────────────────────────────────

/**
 * Encode a Uint8Array to a base64 string.
 * Works in both Node.js (Buffer.from) and browser (btoa + typed arrays).
 */
function _b64encode(bytes) {
  if (typeof Buffer !== 'undefined') {
    return Buffer.from(bytes).toString('base64');
  }
  let binary = '';
  for (let i = 0; i < bytes.length; i++) binary += String.fromCharCode(bytes[i]);
  return btoa(binary);
}

/**
 * Decode a base64 string to a Uint8Array.
 */
function _b64decode(str) {
  if (typeof Buffer !== 'undefined') {
    const buf = Buffer.from(str, 'base64');
    return new Uint8Array(buf.buffer, buf.byteOffset, buf.byteLength);
  }
  const binary = atob(str);
  const bytes = new Uint8Array(binary.length);
  for (let i = 0; i < binary.length; i++) bytes[i] = binary.charCodeAt(i);
  return bytes;
}

// ── Deep clone ─────────────────────────────────────────────────────────────────

function _deepClone(obj) {
  return JSON.parse(JSON.stringify(obj));
}

// ── VirtFS ─────────────────────────────────────────────────────────────────────

export class VirtFS {
  /**
   * @param {object} tree  The initial JSON filesystem tree.  Defaults to an empty root.
   */
  constructor(tree = { '/': {} }) {
    this._tree = tree;
    // Binary cursors: Map<absolutePath, number>
    this._cursors = new Map();
    // Current working directory
    this._cwd = '/';
  }

  /** Parse a JSON string into a VirtFS. */
  static fromJSON(json) {
    return new VirtFS(JSON.parse(json));
  }

  /** Serialise the current state as a plain tree object (not a string). */
  toJSON() {
    return _deepClone(this._tree);
  }

  // ── Snapshot / restore ──────────────────────────────────────────────────────

  /** Return a deep clone of the tree for later restoration. */
  snapshot() {
    return _deepClone(this._tree);
  }

  /** Replace the current tree with a prior snapshot and clear all cursors. */
  restore(snapshot) {
    this._tree = _deepClone(snapshot);
    this._cursors.clear();
  }

  // ── Working directory ───────────────────────────────────────────────────────

  get cwd() { return this._cwd; }
  set cwd(path) { this._cwd = this.resolve(path); }

  chdir(path) { this._cwd = this.resolve(path); }

  // ── Path resolution ─────────────────────────────────────────────────────────

  /**
   * Normalise a path:
   *   - Backslashes → forward slashes
   *   - Relative → prepend cwd
   *   - Collapse `//`, resolve `.` and `..`
   *   - Strip trailing slash (except for '/')
   */
  resolve(path) {
    path = path.replace(/\\/g, '/');
    if (!path.startsWith('/')) {
      path = this._cwd.replace(/\/$/, '') + '/' + path;
    }
    const parts = path.split('/').filter(Boolean);
    const resolved = [];
    for (const p of parts) {
      if (p === '.') continue;
      if (p === '..') { resolved.pop(); }
      else { resolved.push(p); }
    }
    return '/' + resolved.join('/');
  }

  /**
   * Walk the tree to `path` and return `{ parent, name, node }`, or `null` if not found.
   * `parent` is the directory object containing `name`.
   * For the root `/`, returns `{ parent: null, name: null, node: tree['/'] }`.
   */
  _navigate(path) {
    path = this.resolve(path);
    const root = this._tree['/'];
    if (root === undefined) return null;
    if (path === '/') return { parent: null, name: null, node: root };

    const parts = path.slice(1).split('/');
    let current = root;
    for (let i = 0; i < parts.length - 1; i++) {
      const part = parts[i];
      if (!Object.hasOwn(current, part) || current[part]?.$type !== undefined) return null;
      current = current[part];
    }
    const last = parts[parts.length - 1];
    if (!Object.hasOwn(current, last)) {
      return { parent: current, name: last, node: undefined };
    }
    return { parent: current, name: last, node: current[last] };
  }

  // ── Read operations ─────────────────────────────────────────────────────────

  exists(path) {
    const nav = this._navigate(path);
    return nav !== null && nav.node !== undefined;
  }

  isFile(path) {
    const nav = this._navigate(path);
    if (!nav || nav.node === undefined) return false;
    return nav.node?.$type === 'text' || nav.node?.$type === 'binary';
  }

  isDirectory(path) {
    const nav = this._navigate(path);
    if (!nav || nav.node === undefined) return false;
    return nav.node?.$type === undefined;
  }

  /**
   * @returns {{ type: string, size: number } | null}
   */
  stat(path) {
    const nav = this._navigate(path);
    if (!nav || nav.node === undefined) return null;
    const node = nav.node;
    if (node.$type === 'text') {
      // Size in UTF-8 bytes
      const size = new TextEncoder().encode(node.$content).length;
      return { type: 'text', size };
    }
    if (node.$type === 'binary') {
      const bytes = _b64decode(node.$content);
      return { type: 'binary', size: bytes.length };
    }
    // directory
    return { type: 'directory', size: 0 };
  }

  /** @returns {string | null} */
  readText(path) {
    const nav = this._navigate(path);
    if (!nav || nav.node === undefined) return null;
    if (nav.node.$type !== 'text') return null;
    return nav.node.$content;
  }

  /** @returns {Uint8Array | null} */
  readBinary(path) {
    const nav = this._navigate(path);
    if (!nav || nav.node === undefined) return null;
    if (nav.node.$type !== 'binary') return null;
    return _b64decode(nav.node.$content);
  }

  /** @returns {string[]} Entry names (not full paths), or [] for missing/non-directory. */
  readdir(path) {
    const nav = this._navigate(path);
    if (!nav || nav.node === undefined) return [];
    if (nav.node.$type !== undefined) return [];  // not a directory
    return Object.keys(nav.node).filter(k => !k.startsWith('$'));
  }

  // ── Write operations ────────────────────────────────────────────────────────

  /**
   * Write a text file, creating all parent directories as needed.
   */
  writeText(path, content) {
    path = this.resolve(path);
    this._mkdirAllInternal(path.substring(0, path.lastIndexOf('/')) || '/');
    const nav = this._navigate(path);
    if (!nav) throw new Error(`Cannot create file at ${path}: path error`);
    nav.parent[nav.name] = { $type: 'text', $content: content };
    this._cursors.delete(path);
  }

  /**
   * Write a binary file (Uint8Array), creating all parent directories as needed.
   */
  writeBinary(path, bytes) {
    path = this.resolve(path);
    this._mkdirAllInternal(path.substring(0, path.lastIndexOf('/')) || '/');
    const nav = this._navigate(path);
    if (!nav) throw new Error(`Cannot create binary file at ${path}: path error`);
    nav.parent[nav.name] = { $type: 'binary', $content: _b64encode(bytes) };
    this._cursors.delete(path);
  }

  /**
   * Create a single directory level.  Throws if the parent does not exist.
   */
  mkdir(path) {
    path = this.resolve(path);
    if (path === '/') return;  // root always exists
    const parentPath = path.substring(0, path.lastIndexOf('/')) || '/';
    const parentNav = this._navigate(parentPath);
    if (!parentNav || parentNav.node === undefined || parentNav.node.$type !== undefined) {
      throw new Error(`mkdir: parent directory does not exist: ${parentPath}`);
    }
    const name = path.substring(path.lastIndexOf('/') + 1);
    if (!Object.hasOwn(parentNav.node, name)) {
      parentNav.node[name] = {};
    }
  }

  /**
   * Create a directory and all its ancestors.
   */
  mkdirAll(path) {
    this._mkdirAllInternal(this.resolve(path));
  }

  _mkdirAllInternal(path) {
    if (path === '/') return;
    const parts = path.slice(1).split('/').filter(Boolean);
    let current = this._tree['/'];
    for (const part of parts) {
      if (!Object.hasOwn(current, part)) {
        current[part] = {};
      } else if (current[part]?.$type !== undefined) {
        throw new Error(`mkdirAll: path component is a file: ${part}`);
      }
      current = current[part];
    }
  }

  /**
   * Delete a file.  Throws if path does not exist or is a directory.
   */
  delete(path) {
    path = this.resolve(path);
    const nav = this._navigate(path);
    if (!nav || nav.node === undefined) throw new Error(`delete: not found: ${path}`);
    if (nav.node.$type === undefined) throw new Error(`delete: is a directory: ${path}`);
    delete nav.parent[nav.name];
    this._cursors.delete(path);
  }

  /**
   * Delete an empty directory.  Throws if not empty or not a directory.
   */
  deleteDir(path) {
    path = this.resolve(path);
    if (path === '/') throw new Error('deleteDir: cannot delete root');
    const nav = this._navigate(path);
    if (!nav || nav.node === undefined) throw new Error(`deleteDir: not found: ${path}`);
    if (nav.node.$type !== undefined) throw new Error(`deleteDir: not a directory: ${path}`);
    const entries = Object.keys(nav.node).filter(k => !k.startsWith('$'));
    if (entries.length > 0) throw new Error(`deleteDir: directory not empty: ${path}`);
    delete nav.parent[nav.name];
  }

  /**
   * Move / rename a file or directory.
   */
  move(from, to) {
    from = this.resolve(from);
    to = this.resolve(to);
    const srcNav = this._navigate(from);
    if (!srcNav || srcNav.node === undefined) throw new Error(`move: source not found: ${from}`);

    const dstParent = to.substring(0, to.lastIndexOf('/')) || '/';
    this._mkdirAllInternal(dstParent);
    const dstNav = this._navigate(to);
    if (!dstNav) throw new Error(`move: destination path error: ${to}`);

    dstNav.parent[dstNav.name] = srcNav.node;
    delete srcNav.parent[srcNav.name];

    // Update cursors: move any cursor keyed by `from` to `to`
    if (this._cursors.has(from)) {
      this._cursors.set(to, this._cursors.get(from));
      this._cursors.delete(from);
    }
  }

  // ── Binary cursor ───────────────────────────────────────────────────────────

  seek(path, pos) {
    path = this.resolve(path);
    this._cursors.set(path, pos);
  }

  getCursor(path) {
    path = this.resolve(path);
    return this._cursors.get(path) ?? 0;
  }

  /** Read `n` bytes from the cursor position; advance the cursor. */
  readBytes(path, n) {
    path = this.resolve(path);
    const all = this.readBinary(path);
    if (!all) return null;
    const cursor = this._cursors.get(path) ?? 0;
    const slice = all.slice(cursor, cursor + n);
    this._cursors.set(path, cursor + slice.length);
    return slice;
  }

  /** Write bytes at the cursor position, extending/overwriting as needed; advance the cursor. */
  writeBytes(path, bytes) {
    path = this.resolve(path);
    const existing = this.readBinary(path) ?? new Uint8Array(0);
    const cursor = this._cursors.get(path) ?? 0;
    const end = cursor + bytes.length;
    const newLen = Math.max(existing.length, end);
    const updated = new Uint8Array(newLen);
    updated.set(existing);
    updated.set(bytes, cursor);
    // Store back (bypass writeText parent-dir logic — file already exists or we just read it)
    const nav = this._navigate(path);
    if (!nav || nav.node === undefined) {
      // File doesn't exist yet — create it
      this.writeBinary(path, updated);
    } else {
      nav.parent[nav.name] = { $type: 'binary', $content: _b64encode(updated) };
    }
    this._cursors.set(path, end);
  }
}
