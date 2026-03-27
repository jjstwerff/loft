// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

/**
 * W1.12 — Unit tests for the LayeredFS class.
 *
 * Run:
 *   node tests/wasm/layered-fs.test.mjs
 */

import { test, assert, run } from './harness.mjs';
import { LayeredFS } from './layered-fs.mjs';

// ── Helpers ───────────────────────────────────────────────────────────────────

function makeBase() {
  return {
    '/': {
      'examples': {
        'hello.loft': { '$type': 'text', '$content': 'fn main() { println("hi") }' },
        'math.loft':  { '$type': 'text', '$content': 'fn add(a: integer, b: integer) -> integer { a + b }' }
      },
      'lib': {
        '01_code.loft': { '$type': 'text', '$content': '// stdlib' }
      }
    }
  };
}

// ── Tests from the WASM.md spec ───────────────────────────────────────────────

test('user edit shadows base file', () => {
  const fs = new LayeredFS(makeBase());

  // unmodified — reads from base
  assert(fs.readText('/examples/hello.loft').includes('hi'));
  assert(!fs.isModified('/examples/hello.loft'));

  // user edits — goes to delta
  fs.writeText('/examples/hello.loft', 'fn main() { println("bye") }');
  assert(fs.readText('/examples/hello.loft').includes('bye'));
  assert(fs.isModified('/examples/hello.loft'));

  // delta is small — only the modified file
  const delta = fs.getDelta();
  assert(Object.keys(delta.files).length === 1);

  // reset brings back original
  fs.resetToBase();
  assert(fs.readText('/examples/hello.loft').includes('hi'));
});

test('delete base file is tracked in delta', () => {
  const base = {
    '/': {
      'examples': {
        'a.loft': { '$type': 'text', '$content': 'fn a() {}' },
        'b.loft': { '$type': 'text', '$content': 'fn b() {}' }
      }
    }
  };
  const fs = new LayeredFS(base);

  fs.delete('/examples/a.loft');
  assert(!fs.exists('/examples/a.loft'));
  assert(fs.exists('/examples/b.loft'));
  assert.deepEqual(fs.readdir('/examples'), ['b.loft']);

  // delta tracks only the deletion — no copy of b.loft
  const delta = fs.getDelta();
  assert(delta.deleted.includes('/examples/a.loft'));
  assert(Object.keys(delta.files).length === 0);
});

test('new user file coexists with base', () => {
  const base = {
    '/': {
      'examples': {
        'hello.loft': { '$type': 'text', '$content': 'fn main() {}' }
      }
    }
  };
  const fs = new LayeredFS(base);

  fs.writeText('/my-project/main.loft', 'fn main() { println("mine") }');
  assert(fs.exists('/my-project/main.loft'));
  assert(fs.exists('/examples/hello.loft'));  // base still visible
  assert.deepEqual(fs.readdir('/').sort(), ['examples', 'my-project']);
});

test('delta serialise and reload', () => {
  const base = {
    '/': {
      'examples': {
        'a.loft': { '$type': 'text', '$content': 'original' }
      }
    }
  };
  const fs = new LayeredFS(base);
  fs.writeText('/examples/a.loft', 'modified');
  fs.writeText('/new.loft', 'brand new');

  // simulate save/reload cycle
  const deltaJson = JSON.stringify(fs.getDelta());
  const fs2 = new LayeredFS(base, JSON.parse(deltaJson));

  assert(fs2.readText('/examples/a.loft') === 'modified');
  assert(fs2.readText('/new.loft') === 'brand new');
});

// ── Additional tests ──────────────────────────────────────────────────────────

test('base is not mutated by writes', () => {
  const base = makeBase();
  const fs = new LayeredFS(base);
  fs.writeText('/examples/hello.loft', 'new content');

  // base object itself must not be changed
  assert(base['/'].examples['hello.loft'].$content.includes('hi'));
});

test('empty filesystem has root', () => {
  const fs = new LayeredFS({ '/': {} });
  assert(fs.isDirectory('/'));
  assert.deepEqual(fs.readdir('/'), []);
});

test('isFile and isDirectory for base and delta entries', () => {
  const fs = new LayeredFS(makeBase());
  assert(fs.isFile('/examples/hello.loft'));
  assert(fs.isDirectory('/examples'));
  assert(fs.isDirectory('/'));
  assert(!fs.isFile('/examples'));
  assert(!fs.isDirectory('/examples/hello.loft'));

  fs.writeText('/new/file.loft', 'content');
  assert(fs.isFile('/new/file.loft'));
  assert(fs.isDirectory('/new'));
  assert(!fs.isFile('/new'));
});

test('deleted base entry is no longer a file', () => {
  const fs = new LayeredFS(makeBase());
  fs.delete('/examples/hello.loft');
  assert(!fs.isFile('/examples/hello.loft'));
  assert(!fs.exists('/examples/hello.loft'));
});

test('stat reflects delta content', () => {
  const fs = new LayeredFS(makeBase());
  fs.writeText('/examples/hello.loft', 'hello');  // 5 bytes
  const s = fs.stat('/examples/hello.loft');
  assert(s !== null);
  assert(s.type === 'text');
  assert(s.size === 5);
});

test('binary roundtrip through delta', () => {
  const fs = new LayeredFS({ '/': {} });
  const data = new Uint8Array([10, 20, 30, 255]);
  fs.writeBinary('/data.bin', data);
  assert(fs.isFile('/data.bin'));
  assert.deepEqual(fs.readBinary('/data.bin'), data);
  const s = fs.stat('/data.bin');
  assert(s.type === 'binary' && s.size === 4);
});

test('snapshot and restore cover delta state', () => {
  const fs = new LayeredFS(makeBase());
  fs.writeText('/examples/hello.loft', 'modified');
  const snap = fs.snapshot();

  fs.writeText('/examples/hello.loft', 'further modified');
  fs.writeText('/extra.loft', 'leaked');

  fs.restore(snap);
  assert(fs.readText('/examples/hello.loft') === 'modified');
  assert(!fs.exists('/extra.loft'));
});

test('readdir merges base and delta entries, excludes deleted', () => {
  const base = {
    '/': {
      'dir': {
        'base-a.loft': { '$type': 'text', '$content': 'a' },
        'base-b.loft': { '$type': 'text', '$content': 'b' }
      }
    }
  };
  const fs = new LayeredFS(base);
  fs.writeText('/dir/user-c.loft', 'c');
  fs.delete('/dir/base-b.loft');

  const entries = fs.readdir('/dir').sort();
  assert.deepEqual(entries, ['base-a.loft', 'user-c.loft']);
});

test('modifiedPaths lists delta file paths', () => {
  const fs = new LayeredFS(makeBase());
  assert.deepEqual(fs.modifiedPaths(), []);

  fs.writeText('/examples/hello.loft', 'changed');
  fs.writeText('/new.loft', 'new');

  const paths = fs.modifiedPaths().sort();
  assert.deepEqual(paths, ['/examples/hello.loft', '/new.loft']);
});

test('isDeleted reports deleted base files', () => {
  const fs = new LayeredFS(makeBase());
  assert(!fs.isDeleted('/examples/hello.loft'));
  fs.delete('/examples/hello.loft');
  assert(fs.isDeleted('/examples/hello.loft'));
  assert(!fs.isDeleted('/examples/math.loft'));
});

test('readText returns null for deleted base file', () => {
  const fs = new LayeredFS(makeBase());
  fs.delete('/examples/hello.loft');
  assert(fs.readText('/examples/hello.loft') === null);
});

test('re-writing a deleted file restores it', () => {
  const fs = new LayeredFS(makeBase());
  fs.delete('/examples/hello.loft');
  assert(!fs.exists('/examples/hello.loft'));

  fs.writeText('/examples/hello.loft', 'restored');
  assert(fs.exists('/examples/hello.loft'));
  assert(fs.readText('/examples/hello.loft') === 'restored');
  assert(!fs.isDeleted('/examples/hello.loft'));
});

test('mkdir in delta works', () => {
  const fs = new LayeredFS({ '/': {} });
  fs.mkdir('/');  // root always ok
  fs.writeText('/parent/.keep', '');
  fs.mkdir('/parent/child');
  assert(fs.isDirectory('/parent/child'));
});

test('mkdir without parent throws', () => {
  const fs = new LayeredFS({ '/': {} });
  assert.throws(() => fs.mkdir('/no/parent/dir'));
});

test('mkdirAll creates full delta path', () => {
  const fs = new LayeredFS({ '/': {} });
  fs.mkdirAll('/a/b/c/d');
  assert(fs.isDirectory('/a/b/c/d'));
  assert(fs.isDirectory('/a/b'));
});

test('getDelta / setDelta round-trip', () => {
  const fs = new LayeredFS(makeBase());
  fs.writeText('/x.loft', 'x');
  const d = fs.getDelta();
  assert('/x.loft' in d.files);

  const fs2 = new LayeredFS(makeBase());
  fs2.setDelta(d);
  assert(fs2.readText('/x.loft') === 'x');
});

// ── Run ────────────────────────────────────────────────────────────────────────

const failed = await run();
process.exit(failed > 0 ? 1 : 0);
