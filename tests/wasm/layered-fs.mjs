// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

/**
 * W1.12 — Layered filesystem: immutable base tree + mutable delta overlay.
 *
 * The base tree ships with the IDE (examples, docs, default stdlib).  It is
 * read-only — never written to.  All user edits land in the delta which is
 * persisted to localStorage / IndexedDB.
 *
 * Delta format:
 *   {
 *     files:   { "/abs/path": { "$type": "text"|"binary", "$content": "..." } },
 *     deleted: [ "/abs/path", ... ],
 *     dirs:    [ "/abs/path", ... ]
 *   }
 *
 * Read priority: delta (wins) → base fallback.
 * Writes always go to delta; base is never mutated.
 */

'use strict';

import { VirtFS } from './virt-fs.mjs';

// ── Base64 helpers (duplicated from virt-fs.mjs for module independence) ───────

function _b64encode(bytes) {
  if (typeof Buffer !== 'undefined') {
    return Buffer.from(bytes).toString('base64');
  }
  let binary = '';
  for (let i = 0; i < bytes.length; i++) binary += String.fromCharCode(bytes[i]);
  return btoa(binary);
}

function _b64decode(str) {
  if (str === '') return new Uint8Array(0);
  if (typeof Buffer !== 'undefined') {
    const buf = Buffer.from(str, 'base64');
    return new Uint8Array(buf.buffer, buf.byteOffset, buf.byteLength);
  }
  const binary = atob(str);
  const bytes = new Uint8Array(binary.length);
  for (let i = 0; i < binary.length; i++) bytes[i] = binary.charCodeAt(i);
  return bytes;
}

// ── LayeredFS ──────────────────────────────────────────────────────────────────

export class LayeredFS extends VirtFS {
  /**
   * @param {object} baseTree  The immutable base tree (JSON tree object).
   * @param {object|null} delta  Prior delta to restore, or null to start fresh.
   */
  constructor(baseTree, delta = null) {
    super(baseTree);                         // VirtFS owns _tree = baseTree (read-only)
    this._base = baseTree;                   // keep reference for reset
    this._delta = delta ?? { files: {}, deleted: [], dirs: [] };
  }

  // ── Snapshot / restore — tracks delta state only ────────────────────────────
  // (base never changes, so only delta needs to be captured)

  snapshot() {
    return JSON.parse(JSON.stringify(this._delta));
  }

  restore(snap) {
    this._delta = JSON.parse(JSON.stringify(snap));
    this._cursors.clear();
  }

  // ── Read overrides: delta wins, then base ───────────────────────────────────

  exists(path) {
    path = this.resolve(path);
    if (this._delta.deleted.includes(path)) return false;
    if (path in this._delta.files) return true;
    if (this._delta.dirs.includes(path)) return true;
    return super.exists(path);
  }

  isFile(path) {
    path = this.resolve(path);
    if (this._delta.deleted.includes(path)) return false;
    if (path in this._delta.files) return true;
    if (this._delta.dirs.includes(path)) return false;
    return super.isFile(path);
  }

  isDirectory(path) {
    path = this.resolve(path);
    if (this._delta.deleted.includes(path)) return false;
    if (path in this._delta.files) return false;
    if (this._delta.dirs.includes(path)) return true;
    return super.isDirectory(path);
  }

  stat(path) {
    path = this.resolve(path);
    if (this._delta.deleted.includes(path)) return null;
    const df = this._delta.files[path];
    if (df) {
      if (df.$type === 'text') {
        const size = new TextEncoder().encode(df.$content).length;
        return { type: 'text', size };
      }
      if (df.$type === 'binary') {
        return { type: 'binary', size: _b64decode(df.$content).length };
      }
    }
    if (this._delta.dirs.includes(path)) return { type: 'directory', size: 0 };
    return super.stat(path);
  }

  readText(path) {
    path = this.resolve(path);
    if (this._delta.deleted.includes(path)) return null;
    const df = this._delta.files[path];
    if (df) return df.$type === 'text' ? df.$content : null;
    return super.readText(path);
  }

  readBinary(path) {
    path = this.resolve(path);
    if (this._delta.deleted.includes(path)) return null;
    const df = this._delta.files[path];
    if (df) return df.$type === 'binary' ? _b64decode(df.$content) : null;
    return super.readBinary(path);
  }

  readdir(path) {
    path = this.resolve(path);
    const entries = new Set(super.readdir(path) ?? []);

    // Add delta files whose parent is `path`
    for (const p of Object.keys(this._delta.files)) {
      const dir = p.substring(0, p.lastIndexOf('/')) || '/';
      if (dir === path) entries.add(p.substring(p.lastIndexOf('/') + 1));
    }

    // Add delta dirs whose parent is `path`
    for (const d of this._delta.dirs) {
      const parent = d.substring(0, d.lastIndexOf('/')) || '/';
      if (parent === path) entries.add(d.substring(d.lastIndexOf('/') + 1));
    }

    // Remove delta-deleted entries in `path`
    for (const del of this._delta.deleted) {
      const dir = del.substring(0, del.lastIndexOf('/')) || '/';
      if (dir === path) entries.delete(del.substring(del.lastIndexOf('/') + 1));
    }

    return [...entries];
  }

  // ── Write overrides: always go to delta ─────────────────────────────────────

  writeText(path, content) {
    path = this.resolve(path);
    this._ensureParentDirs(path);
    this._delta.files[path] = { $type: 'text', $content: content };
    this._delta.deleted = this._delta.deleted.filter(p => p !== path);
    this._cursors.delete(path);
  }

  writeBinary(path, bytes) {
    path = this.resolve(path);
    this._ensureParentDirs(path);
    this._delta.files[path] = { $type: 'binary', $content: _b64encode(bytes) };
    this._delta.deleted = this._delta.deleted.filter(p => p !== path);
    this._cursors.delete(path);
  }

  mkdir(path) {
    path = this.resolve(path);
    if (path === '/') return;
    const parentPath = path.substring(0, path.lastIndexOf('/')) || '/';
    if (!this.isDirectory(parentPath)) {
      throw new Error(`mkdir: parent directory does not exist: ${parentPath}`);
    }
    if (!this.isDirectory(path) && !this.isFile(path)) {
      this._delta.dirs.push(path);
      this._delta.deleted = this._delta.deleted.filter(p => p !== path);
    }
  }

  mkdirAll(path) {
    this._mkdirAllDelta(this.resolve(path));
  }

  delete(path) {
    path = this.resolve(path);
    delete this._delta.files[path];
    if (!this._delta.deleted.includes(path)) {
      this._delta.deleted.push(path);
    }
    this._cursors.delete(path);
  }

  // ── Internal helpers ─────────────────────────────────────────────────────────

  _ensureParentDirs(path) {
    const parts = path.slice(1).split('/').filter(Boolean);
    parts.pop();  // remove filename — only create parent dirs
    this._mkdirAllDelta('/' + parts.join('/'));
  }

  _mkdirAllDelta(path) {
    if (path === '/' || path === '') return;
    const parts = path.slice(1).split('/').filter(Boolean);
    let current = '/';
    for (const p of parts) {
      current = current === '/' ? `/${p}` : `${current}/${p}`;
      if (!this.isDirectory(current)) {
        if (this.isFile(current)) throw new Error(`mkdirAll: path component is a file: ${current}`);
        this._delta.dirs.push(current);
        this._delta.deleted = this._delta.deleted.filter(dp => dp !== current);
      }
    }
  }

  // ── Delta persistence ────────────────────────────────────────────────────────

  getDelta()      { return this._delta; }
  setDelta(delta) { this._delta = delta; }

  /**
   * Persist the delta to localStorage (browser-only).
   * @param {string} key  localStorage key (default: 'loft-ide-delta')
   */
  saveDelta(key = 'loft-ide-delta') {
    if (typeof localStorage === 'undefined') {
      throw new Error('saveDelta: localStorage is not available in this environment');
    }
    localStorage.setItem(key, JSON.stringify(this._delta));
  }

  /**
   * Load a previously saved delta from localStorage (browser-only).
   * Returns null if no delta is stored under `key`.
   * @param {string} key
   * @returns {object|null}
   */
  static loadDelta(key = 'loft-ide-delta') {
    if (typeof localStorage === 'undefined') return null;
    const raw = localStorage.getItem(key);
    return raw ? JSON.parse(raw) : null;
  }

  // ── Query helpers ────────────────────────────────────────────────────────────

  /** Discard all user changes and restore the base tree. */
  resetToBase() {
    this._delta = { files: {}, deleted: [], dirs: [] };
    this._cursors.clear();
  }

  /** True if the file at `path` has been added or modified relative to base. */
  isModified(path) {
    return this.resolve(path) in this._delta.files;
  }

  /** True if `path` has been deleted (exists in base but marked deleted in delta). */
  isDeleted(path) {
    return this._delta.deleted.includes(this.resolve(path));
  }

  /** Absolute paths of all files added or modified in the delta. */
  modifiedPaths() {
    return Object.keys(this._delta.files);
  }
}
