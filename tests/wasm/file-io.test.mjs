// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

/**
 * W1.11 — File I/O edge-case tests through the VirtFS host bridge.
 *
 * These tests exercise the VirtFS ↔ loftHost adapter layer for edge cases
 * that bridge.test.mjs does not cover.  They do not require a full WASM build:
 * the host functions are called directly against VirtFS.
 *
 * Run:
 *   node tests/wasm/file-io.test.mjs
 */

import { test, assert, run } from './harness.mjs';
import { createHost } from './host.mjs';

// ── Helpers ───────────────────────────────────────────────────────────────────

/** Build a fresh host + VirtFS with an empty root. */
function emptyHost() {
  return createHost({ '/': {} });
}

// ── Tests ──────────────────────────────────────────────────────────────────────

test('read nonexistent file returns null', () => {
  const { host } = emptyHost();
  assert(host.fs_read_text('/no/such/file.txt') === null);
  assert(host.fs_exists('/no/such/file.txt') === false);
});

test('write to nested path auto-creates dirs', () => {
  const { host } = emptyHost();
  const rc = host.fs_write_text('/a/b/c.txt', 'hello');
  assert(rc === 0, `Expected rc=0, got ${rc}`);
  assert(host.fs_exists('/a/b/c.txt'));
  assert(host.fs_read_text('/a/b/c.txt') === 'hello');
  assert(host.fs_is_dir('/a'));
  assert(host.fs_is_dir('/a/b'));
  assert(host.fs_is_file('/a/b/c.txt'));
});

test('delete nonexistent returns error code', () => {
  const { host } = emptyHost();
  const rc = host.fs_delete('/not/there.txt');
  // fs_delete returns 1 (NotFound) for a missing file
  assert(rc !== 0, `Expected non-zero rc for missing file, got ${rc}`);
});

test('move across directories', () => {
  const { host } = emptyHost();
  host.fs_write_text('/src/foo.txt', 'content');
  const rc = host.fs_move('/src/foo.txt', '/dst/foo.txt');
  assert(rc === 0, `Expected rc=0, got ${rc}`);
  assert(!host.fs_exists('/src/foo.txt'));
  assert(host.fs_exists('/dst/foo.txt'));
  assert(host.fs_read_text('/dst/foo.txt') === 'content');
});

test('overwrite existing file', () => {
  const { host } = emptyHost();
  host.fs_write_text('/f.txt', 'first');
  host.fs_write_text('/f.txt', 'second');
  assert(host.fs_read_text('/f.txt') === 'second');
});

test('fs_file_size reflects text write', () => {
  const { host } = emptyHost();
  host.fs_write_text('/f.txt', 'hello');
  const size = host.fs_file_size('/f.txt');
  // 'hello' is 5 UTF-8 bytes
  assert(size === 5, `Expected size=5, got ${size}`);
});

test('fs_file_size returns -1 for missing file', () => {
  const { host } = emptyHost();
  assert(host.fs_file_size('/missing.txt') === -1);
});

test('binary cursor resets on write', () => {
  const { host } = emptyHost();
  const data = new Uint8Array([10, 20, 30, 40]);
  host.fs_write_binary('/b.bin', data);
  host.fs_seek('/b.bin', 2);
  assert(host.fs_get_cursor('/b.bin') === 2);
  // Overwriting resets cursor (writeText/writeBinary clears cursors)
  host.fs_write_binary('/b.bin', new Uint8Array([1, 2, 3]));
  assert(host.fs_get_cursor('/b.bin') === 0);
});

test('binary cursor seek and read advance correctly', () => {
  const { host } = emptyHost();
  host.fs_write_binary('/b.bin', new Uint8Array([10, 20, 30, 40, 50]));
  host.fs_seek('/b.bin', 1);
  const chunk = host.fs_read_bytes('/b.bin', 2);
  assert(chunk instanceof Uint8Array);
  assert(chunk[0] === 20 && chunk[1] === 30, `Got ${Array.from(chunk)}`);
  assert(host.fs_get_cursor('/b.bin') === 3);
});

test('fs_list_dir returns entry names', () => {
  const { host } = emptyHost();
  host.fs_write_text('/d/a.txt', 'a');
  host.fs_write_text('/d/b.txt', 'b');
  const entries = host.fs_list_dir('/d').sort();
  assert.deepEqual(entries, ['a.txt', 'b.txt']);
});

test('mkdir_all creates full path', () => {
  const { host } = emptyHost();
  const rc = host.fs_mkdir_all('/x/y/z');
  assert(rc === 0, `Expected rc=0, got ${rc}`);
  assert(host.fs_is_dir('/x/y/z'));
});

test('mkdir without parent returns error', () => {
  const { host } = emptyHost();
  const rc = host.fs_mkdir('/no/parent/dir');
  assert(rc !== 0, `Expected non-zero rc for missing parent`);
});

test('fs_cwd returns current directory', () => {
  const { host } = emptyHost();
  assert(typeof host.fs_cwd() === 'string');
  assert(host.fs_cwd() === '/');
});

test('storage_get / storage_set / storage_remove round-trip', () => {
  const { host } = emptyHost();
  assert(host.storage_get('k') === null);
  host.storage_set('k', 'value');
  assert(host.storage_get('k') === 'value');
  host.storage_remove('k');
  assert(host.storage_get('k') === null);
});

test('env_variable returns null for unknown key by default', () => {
  const { host } = emptyHost();
  assert(host.env_variable('NONEXISTENT_VAR') === null);
});

test('env_variable returns option value when configured', () => {
  const { host } = createHost({ '/': {} }, { env: { MY_VAR: 'hello' } });
  assert(host.env_variable('MY_VAR') === 'hello');
});

// ── Run ────────────────────────────────────────────────────────────────────────

const failed = await run();
process.exit(failed > 0 ? 1 : 0);
