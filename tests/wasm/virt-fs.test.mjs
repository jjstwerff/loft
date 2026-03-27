// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

/**
 * W1.10 — Unit tests for the VirtFS class.
 *
 * Run directly:
 *   node tests/wasm/virt-fs.test.mjs
 */

import { test, assert, run } from './harness.mjs';
import { VirtFS } from './virt-fs.mjs';

// ── Tests ──────────────────────────────────────────────────────────────────────

test('empty filesystem has root', () => {
  const fs = new VirtFS();
  assert(fs.isDirectory('/'));
  assert.deepEqual(fs.readdir('/'), []);
});

test('writeText creates file and parent dirs', () => {
  const fs = new VirtFS();
  fs.writeText('/a/b/c.txt', 'hello');
  assert(fs.isDirectory('/a'));
  assert(fs.isDirectory('/a/b'));
  assert(fs.isFile('/a/b/c.txt'));
  assert(fs.readText('/a/b/c.txt') === 'hello');
});

test('readdir lists entries', () => {
  const fs = new VirtFS({
    '/': {
      'x.txt': { '$type': 'text', '$content': 'x' },
      'y.txt': { '$type': 'text', '$content': 'y' },
      'sub': {}
    }
  });
  const entries = fs.readdir('/').sort();
  assert.deepEqual(entries, ['sub', 'x.txt', 'y.txt']);
});

test('delete removes file', () => {
  const fs = new VirtFS();
  fs.writeText('/f.txt', 'data');
  fs.delete('/f.txt');
  assert(!fs.exists('/f.txt'));
});

test('move renames file', () => {
  const fs = new VirtFS();
  fs.writeText('/old.txt', 'content');
  fs.move('/old.txt', '/new.txt');
  assert(!fs.exists('/old.txt'));
  assert(fs.readText('/new.txt') === 'content');
});

test('mkdir fails without parent', () => {
  const fs = new VirtFS();
  assert.throws(() => fs.mkdir('/a/b/c'));
});

test('mkdirAll creates full path', () => {
  const fs = new VirtFS();
  fs.mkdirAll('/a/b/c');
  assert(fs.isDirectory('/a/b/c'));
});

test('stat returns size for text files', () => {
  const fs = new VirtFS();
  fs.writeText('/f.txt', 'hello');     // 5 bytes UTF-8
  assert(fs.stat('/f.txt').size === 5);
  assert(fs.stat('/f.txt').type === 'text');
});

test('binary roundtrip', () => {
  const fs = new VirtFS();
  const data = new Uint8Array([1, 2, 3, 255]);
  fs.writeBinary('/b.bin', data);
  const out = fs.readBinary('/b.bin');
  assert.deepEqual(out, data);
  assert(fs.stat('/b.bin').type === 'binary');
});

test('binary cursor seek and read', () => {
  const fs = new VirtFS();
  fs.writeBinary('/b.bin', new Uint8Array([10, 20, 30, 40, 50]));
  fs.seek('/b.bin', 2);
  const chunk = fs.readBytes('/b.bin', 2);
  assert.deepEqual(chunk, new Uint8Array([30, 40]));
  assert(fs.getCursor('/b.bin') === 4);
});

test('snapshot and restore isolate mutations', () => {
  const fs = new VirtFS();
  fs.writeText('/keep.txt', 'original');
  const snap = fs.snapshot();

  fs.writeText('/keep.txt', 'modified');
  fs.writeText('/extra.txt', 'leaked');

  fs.restore(snap);
  assert(fs.readText('/keep.txt') === 'original');
  assert(!fs.exists('/extra.txt'));
});

test('resolve normalises paths', () => {
  const fs = new VirtFS();
  assert(fs.resolve('/a/b/../c/./d') === '/a/c/d');
  assert(fs.resolve('/a//b') === '/a/b');
  assert(fs.resolve('/a/b/') === '/a/b');
});

test('toJSON and fromJSON roundtrip', () => {
  const fs = new VirtFS();
  fs.writeText('/a.txt', 'hello');
  fs.mkdirAll('/sub/dir');
  fs.writeBinary('/sub/b.bin', new Uint8Array([42]));

  const json = JSON.stringify(fs.toJSON());
  const fs2 = VirtFS.fromJSON(json);

  assert(fs2.readText('/a.txt') === 'hello');
  assert(fs2.isDirectory('/sub/dir'));
  assert.deepEqual(fs2.readBinary('/sub/b.bin'), new Uint8Array([42]));
});

// ── Run ────────────────────────────────────────────────────────────────────────

const failed = await run();
process.exit(failed > 0 ? 1 : 0);
