# WASM Runtime — Virtual Filesystem, Host Bridges, and Node.js Testing

## Contents
- [Overview](#overview)
- [JSON Virtual Filesystem](#json-virtual-filesystem)
- [Layered Filesystem — Base Tree + Delta Overlay](#layered-filesystem--base-tree--delta-overlay)
- [Host Bridge API](#host-bridge-api)
- [Node.js Test Harness](#nodejs-test-harness)
- [Browser vs Node.js Host Comparison](#browser-vs-nodejs-host-comparison)
- [Implementation Notes](#implementation-notes)
- [See also](#see-also)

---

## Overview

When the loft interpreter runs as a WASM module (via the `wasm` Cargo feature), it
cannot access the real filesystem, system clock, or OS random source. Instead, it calls
out to the JavaScript host through `wasm-bindgen` extern functions grouped under a
`loftHost` namespace. The host provides concrete implementations — browser APIs in
production, in-memory fakes in tests.

This document covers three things:
1. A **JSON virtual filesystem** that gives loft programs realistic file/directory
   behaviour without touching disk.
2. The **host bridge API** for random, time, environment, and storage.
3. A **Node.js test harness** that wires everything together for automated testing.

---

## JSON Virtual Filesystem

### Design goals

- Support the full loft `File` API: `file()`, `content()`, `lines()`, `write()`,
  `files()`, `exists()`, `delete()`, `move()`, `mkdir()`, `mkdir_all()`, binary
  read/write, `seek`, `f#size`, `f#exists`, `f#next`.
- Represent the entire filesystem as a single JSON tree — serialisable, inspectable,
  diffable.
- Allow snapshot/restore for test isolation.
- Run identically in browser (IndexedDB-backed) and Node.js (in-memory).

### Tree structure

```jsonc
{
  "/": {                          // root directory
    "home": {                     // directory node: value is an object
      "user": {
        "project": {
          "main.loft": {          // text file node
            "$type": "text",
            "$content": "fn main() { println(\"hello\") }"
          },
          "data.bin": {           // binary file node
            "$type": "binary",
            "$content": "AQID/w=="  // base64-encoded bytes
          },
          "src": {                // subdirectory — nested object
            "lib.loft": {
              "$type": "text",
              "$content": "pub fn add(a: integer, b: integer) -> integer { a + b }"
            }
          }
        }
      }
    },
    "tmp": {}                     // empty directory
  }
}
```

**Conventions:**
- A key whose value is a plain `{}` or contains nested keys (without `$type`) is a
  **directory**.
- A key whose value is `{ "$type": "text", "$content": "..." }` is a **text file**.
- A key whose value is `{ "$type": "binary", "$content": "<base64>" }` is a **binary
  file**.  Content is base64-encoded.
- Special keys always start with `$` — no loft filename may start with `$`.

### VirtFS class

```js
export class VirtFS {
  // --- construction ---
  constructor(tree = { "/": {} })     // initialise from a JSON tree
  static fromJSON(json)               // parse a JSON string into a VirtFS
  toJSON()                            // serialise the current state

  // --- snapshot / restore (test isolation) ---
  snapshot()                          // returns a deep-cloned tree
  restore(snapshot)                   // replaces the tree with a prior snapshot

  // --- path resolution ---
  // All paths are absolute. Relative paths are resolved against cwd.
  resolve(path)                       // normalise: remove //, resolve . and ..
  private _navigate(path)             // returns { parent, name, node } or null

  // --- read operations ---
  exists(path) -> boolean
  isFile(path) -> boolean
  isDirectory(path) -> boolean
  stat(path) -> { type, size } | null
  readText(path) -> string | null
  readBinary(path) -> Uint8Array | null
  readdir(path) -> string[]           // entry names (not full paths)

  // --- write operations ---
  writeText(path, content)            // creates parent dirs as needed
  writeBinary(path, bytes)            // bytes: Uint8Array; stored as base64
  mkdir(path)                         // single level, error if parent missing
  mkdirAll(path)                      // recursive
  delete(path)                        // file only
  deleteDir(path)                     // directory (must be empty)
  move(from, to)                      // rename/relocate

  // --- binary cursor (per-file state for seek/read) ---
  // Maintains a Map<path, { cursor: number }> for binary file positions.
  seek(path, pos)
  getCursor(path) -> number
  readBytes(path, n) -> Uint8Array    // reads n bytes from cursor, advances
  writeBytes(path, bytes)             // writes at cursor, advances

  // --- working directory ---
  cwd                                 // current working directory (string)
  chdir(path)
}
```

### Path resolution rules

1. Paths use forward slashes. Backslashes are converted on input.
2. `resolve("/home/user/../user/./project")` → `"/home/user/project"`.
3. Trailing slashes are stripped (except for `"/"`).
4. `resolve("relative/path")` prepends `this.cwd`.

### Internal storage

The tree is a plain JS object. File content is stored as a JS string (text) or
base64 string (binary).  Binary cursors are kept in a separate `Map<string, number>`
keyed by absolute path — cursors reset when the file is written or deleted.

### Example: test setup

```js
const fs = new VirtFS({
  "/": {
    "project": {
      "main.loft": { "$type": "text", "$content": "fn main() { println(\"hi\") }" },
      "data": {
        "names.txt": { "$type": "text", "$content": "alice\nbob\ncharlie" }
      }
    }
  }
});

fs.cwd = "/project";

assert(fs.exists("/project/main.loft"));
assert(fs.isDirectory("/project/data"));
assert.deepEqual(fs.readdir("/project/data"), ["names.txt"]);
assert(fs.readText("/project/data/names.txt") === "alice\nbob\ncharlie");
```

### Example: snapshot isolation in tests

```js
test('write does not leak between tests', () => {
  const snap = fs.snapshot();

  fs.writeText("/project/temp.loft", "fn temp() {}");
  assert(fs.exists("/project/temp.loft"));

  fs.restore(snap);
  assert(!fs.exists("/project/temp.loft"));
});
```

---

## Layered Filesystem — Base Tree + Delta Overlay

### Problem

The Web IDE ships with example programs and user documentation (from `tests/docs/`
and `doc/*.html`). These are **read-only defaults** that every user should see. When
a user edits an example or creates a new file, only the **changes** should be persisted
to localStorage/IndexedDB — not a full copy of the entire default tree.

### Design: two-layer VirtFS

```
┌─────────────────────────────────────────────┐
│              LayeredFS (read path)           │
│                                             │
│  read(path):                                │
│    if delta.exists(path)  → return delta    │
│    if delta.deleted(path) → return null     │
│    if base.exists(path)   → return base     │
│    → return null                            │
│                                             │
│  write(path, content):                      │
│    delta.write(path, content)               │
│    (base is never mutated)                  │
│                                             │
│  delete(path):                              │
│    delta.markDeleted(path)                  │
│    (base entry remains but is shadowed)     │
│                                             │
│  readdir(path):                             │
│    merge base entries + delta entries        │
│    minus delta-deleted entries              │
├─────────────────────────────────────────────┤
│  base (immutable)   │  delta (persisted)    │
│  ─────────────────  │  ──────────────────   │
│  Bundled at build   │  localStorage or      │
│  time from:         │  IndexedDB. Starts    │
│  • tests/docs/*.loft│  empty. Only grows    │
│  • doc/*.html       │  when the user edits  │
│  • default/*.loft   │  or creates files.    │
│  Shipped as a       │                       │
│  static JSON file   │  Stored as JSON:      │
│  in ide/assets/     │  { files: {...},      │
│    base-fs.json     │    deleted: [...] }   │
└─────────────────────┴───────────────────────┘
```

### Base tree — `base-fs.json`

Generated at build time by `ide/scripts/build-base-fs.js`:

```jsonc
{
  "/": {
    "examples": {
      "01-hello.loft":    { "$type": "text", "$content": "fn main() {\n  println(\"Hello, world!\")\n}" },
      "06-function.loft": { "$type": "text", "$content": "// Functions example\n..." },
      "07-vector.loft":   { "$type": "text", "$content": "// Vector example\n..." },
      "10-structs.loft":  { "$type": "text", "$content": "..." }
      // ... all tests/docs/*.loft files
    },
    "docs": {
      "language.html":    { "$type": "text", "$content": "<!DOCTYPE html>..." },
      "stdlib.html":      { "$type": "text", "$content": "..." }
      // ... generated doc/*.html files
    },
    "lib": {
      "01_code.loft":     { "$type": "text", "$content": "..." },
      "02_images.loft":   { "$type": "text", "$content": "..." },
      "03_text.loft":     { "$type": "text", "$content": "..." }
    }
  }
}
```

This file is loaded once on startup and never written to.

### Delta format

The delta is a small JSON object persisted to localStorage (or IndexedDB for large
projects):

```jsonc
{
  "files": {
    "/examples/06-function.loft": {        // modified base file
      "$type": "text",
      "$content": "// My edited version\nfn main() { ... }"
    },
    "/my-projects/game/main.loft": {       // new user file
      "$type": "text",
      "$content": "fn main() { ... }"
    }
  },
  "deleted": [
    "/examples/01-hello.loft"              // user deleted this example
  ],
  "dirs": [
    "/my-projects",
    "/my-projects/game"
  ]
}
```

**Storage key**: `loft-ide-delta` (single key for simple projects), or per-project
keys `loft-ide-delta:<project-id>` when multi-project support (M4) is active.

### LayeredFS class

```js
export class LayeredFS extends VirtFS {
  constructor(baseTree, delta = null) {
    super(baseTree);                    // base becomes the read-only layer
    this._base = baseTree;              // keep reference
    this._delta = delta ?? { files: {}, deleted: [], dirs: [] };
  }

  // --- read: delta wins, then base ---
  exists(path) {
    path = this.resolve(path);
    if (this._delta.deleted.includes(path)) return false;
    if (path in this._delta.files) return true;
    if (this._delta.dirs.includes(path)) return true;
    return super.exists(path);           // check base
  }

  readText(path) {
    path = this.resolve(path);
    if (this._delta.deleted.includes(path)) return null;
    const df = this._delta.files[path];
    if (df) return df.$content;
    return super.readText(path);         // fall through to base
  }

  readdir(path) {
    path = this.resolve(path);
    const baseEntries = new Set(super.readdir(path) ?? []);
    // add delta files in this directory
    for (const p of Object.keys(this._delta.files)) {
      const dir = p.substring(0, p.lastIndexOf('/')) || '/';
      if (dir === path) baseEntries.add(p.substring(p.lastIndexOf('/') + 1));
    }
    // add delta dirs
    for (const d of this._delta.dirs) {
      const parent = d.substring(0, d.lastIndexOf('/')) || '/';
      if (parent === path) baseEntries.add(d.substring(d.lastIndexOf('/') + 1));
    }
    // remove deleted entries
    for (const del of this._delta.deleted) {
      const dir = del.substring(0, del.lastIndexOf('/')) || '/';
      if (dir === path) baseEntries.delete(del.substring(del.lastIndexOf('/') + 1));
    }
    return [...baseEntries];
  }

  // --- write: always goes to delta ---
  writeText(path, content) {
    path = this.resolve(path);
    this._ensureParentDirs(path);
    this._delta.files[path] = { $type: 'text', $content: content };
    // remove from deleted if it was there
    this._delta.deleted = this._delta.deleted.filter(p => p !== path);
  }

  delete(path) {
    path = this.resolve(path);
    delete this._delta.files[path];
    if (!this._delta.deleted.includes(path)) {
      this._delta.deleted.push(path);
    }
  }

  // --- persistence ---
  getDelta()          { return this._delta; }
  setDelta(delta)     { this._delta = delta; }

  saveDelta(key = 'loft-ide-delta') {
    localStorage.setItem(key, JSON.stringify(this._delta));
  }

  static loadDelta(key = 'loft-ide-delta') {
    const raw = localStorage.getItem(key);
    return raw ? JSON.parse(raw) : null;
  }

  // --- reset: discard all user changes ---
  resetToBase() {
    this._delta = { files: {}, deleted: [], dirs: [] };
  }

  // --- check if a file has been modified from base ---
  isModified(path) {
    path = this.resolve(path);
    return path in this._delta.files;
  }

  isDeleted(path) {
    return this._delta.deleted.includes(this.resolve(path));
  }

  // --- list all user-modified paths ---
  modifiedPaths() {
    return Object.keys(this._delta.files);
  }
}
```

### Size budget

| Component | Estimated size |
|---|---|
| `base-fs.json` (gzipped) | ~50-100 KB (loft examples + docs) |
| Delta in localStorage | Typically < 50 KB (just edited files) |
| localStorage limit | 5-10 MB — plenty for delta |
| IndexedDB fallback | Needed only if binary data or > 100 files |

Since the base tree is static and cacheable, the service worker (M6) caches it
alongside the WASM module. The delta is tiny and persists in localStorage — no
IndexedDB needed for typical usage.

### Build script — `ide/scripts/build-base-fs.js`

```js
// Reads tests/docs/*.loft, doc/*.html, default/*.loft
// Outputs ide/assets/base-fs.json
import { readFileSync, readdirSync, writeFileSync } from 'fs';

const tree = { "/": { examples: {}, docs: {}, lib: {} } };

// examples from tests/docs/
for (const f of readdirSync('tests/docs').filter(f => f.endsWith('.loft'))) {
  tree["/"].examples[f] = {
    $type: 'text',
    $content: readFileSync(`tests/docs/${f}`, 'utf8')
  };
}

// generated HTML docs
for (const f of readdirSync('doc').filter(f => f.endsWith('.html'))) {
  tree["/"].docs[f] = {
    $type: 'text',
    $content: readFileSync(`doc/${f}`, 'utf8')
  };
}

// default stdlib
for (const f of readdirSync('default').filter(f => f.endsWith('.loft'))) {
  tree["/"].lib[f] = {
    $type: 'text',
    $content: readFileSync(`default/${f}`, 'utf8')
  };
}

writeFileSync('ide/assets/base-fs.json', JSON.stringify(tree));
```

### Browser startup flow

```js
// app.js — startup
const baseTree = await fetch('assets/base-fs.json').then(r => r.json());
const delta = LayeredFS.loadDelta();  // from localStorage, may be null
const fs = new LayeredFS(baseTree, delta);

// Wire the host
globalThis.loftHost = createBrowserHost(fs);

// Auto-save delta on every write (debounced)
let saveTimer;
const originalWrite = fs.writeText.bind(fs);
fs.writeText = (path, content) => {
  originalWrite(path, content);
  clearTimeout(saveTimer);
  saveTimer = setTimeout(() => fs.saveDelta(), 2000);
};
```

### "Reset to default" button

```js
resetButton.onclick = () => {
  if (confirm('Discard all changes and restore examples to original?')) {
    fs.resetToBase();
    fs.saveDelta();    // clears localStorage
    reloadEditor();
  }
};
```

### Node.js testing with layers

```js
import { LayeredFS } from './layered-fs.mjs';

test('user edit shadows base file', () => {
  const base = { "/": { "examples": {
    "hello.loft": { "$type": "text", "$content": "fn main() { println(\"hi\") }" }
  }}};
  const fs = new LayeredFS(base);

  // unmodified — reads from base
  assert(fs.readText('/examples/hello.loft').includes('hi'));
  assert(!fs.isModified('/examples/hello.loft'));

  // user edits — goes to delta
  fs.writeText('/examples/hello.loft', 'fn main() { println("bye") }');
  assert(fs.readText('/examples/hello.loft').includes('bye'));
  assert(fs.isModified('/examples/hello.loft'));

  // delta is small
  const delta = fs.getDelta();
  assert(Object.keys(delta.files).length === 1);

  // reset brings back original
  fs.resetToBase();
  assert(fs.readText('/examples/hello.loft').includes('hi'));
});

test('delete base file is tracked in delta', () => {
  const base = { "/": { "examples": {
    "a.loft": { "$type": "text", "$content": "fn a() {}" },
    "b.loft": { "$type": "text", "$content": "fn b() {}" }
  }}};
  const fs = new LayeredFS(base);

  fs.delete('/examples/a.loft');
  assert(!fs.exists('/examples/a.loft'));
  assert(fs.exists('/examples/b.loft'));    // unaffected
  assert.deepEqual(fs.readdir('/examples'), ['b.loft']);

  // delta only stores the deletion marker, not a copy of b.loft
  const delta = fs.getDelta();
  assert(delta.deleted.includes('/examples/a.loft'));
  assert(Object.keys(delta.files).length === 0);
});

test('new user file coexists with base', () => {
  const base = { "/": { "examples": {
    "hello.loft": { "$type": "text", "$content": "fn main() {}" }
  }}};
  const fs = new LayeredFS(base);

  fs.writeText('/my-project/main.loft', 'fn main() { println("mine") }');
  assert(fs.exists('/my-project/main.loft'));
  assert(fs.exists('/examples/hello.loft'));  // base still visible
  assert.deepEqual(fs.readdir('/').sort(), ['examples', 'my-project']);
});

test('delta serialise and reload', () => {
  const base = { "/": { "examples": {
    "a.loft": { "$type": "text", "$content": "original" }
  }}};
  const fs = new LayeredFS(base);
  fs.writeText('/examples/a.loft', 'modified');
  fs.writeText('/new.loft', 'brand new');

  // simulate save/reload cycle
  const deltaJson = JSON.stringify(fs.getDelta());
  const fs2 = new LayeredFS(base, JSON.parse(deltaJson));

  assert(fs2.readText('/examples/a.loft') === 'modified');
  assert(fs2.readText('/new.loft') === 'brand new');
});
```

---

## Host Bridge API

The WASM module imports functions from a `loftHost` namespace. The host (browser or
Node.js) populates `globalThis.loftHost` before initialising the WASM module.

### Filesystem bridge

These functions map 1:1 to the loft `File` stdlib operations. On the Rust side,
`#[cfg(feature = "wasm")]` branches in `src/state/io.rs` call these instead of
`std::fs`.

```js
globalThis.loftHost = {
  // --- files ---
  fs_exists(path)                     -> boolean,
  fs_read_text(path)                  -> string | null,
  fs_read_binary(path, offset, len)   -> Uint8Array | null,
  fs_write_text(path, content)        -> i32,     // 0=ok, error code otherwise
  fs_write_binary(path, bytes)        -> i32,
  fs_file_size(path)                  -> number,  // bytes, -1 if not found
  fs_delete(path)                     -> i32,
  fs_move(from, to)                   -> i32,
  fs_mkdir(path)                      -> i32,
  fs_mkdir_all(path)                  -> i32,

  // --- directories ---
  fs_list_dir(path)                   -> string[], // entry names
  fs_is_dir(path)                     -> boolean,
  fs_is_file(path)                    -> boolean,

  // --- binary cursor ---
  fs_seek(path, pos),
  fs_read_bytes(path, n)              -> Uint8Array | null,
  fs_write_bytes(path, bytes)         -> i32,
  fs_get_cursor(path)                 -> number,

  // --- paths ---
  fs_cwd()                            -> string,
  fs_user_dir()                       -> string,
  fs_program_dir()                    -> string,
  // ...
};
```

**Return codes** (matching `FileResult` enum):

| Code | Meaning |
|------|---------|
| 0 | `Ok` |
| 1 | `NotFound` |
| 2 | `PermissionDenied` |
| 3 | `IsDirectory` |
| 4 | `NotDirectory` |
| 5 | `Other` |

### Random bridge

```js
globalThis.loftHost = {
  // ...
  random_int(lo, hi)                  -> number,   // integer in [lo, hi]
  random_seed(seed_hi, seed_lo),                    // 64-bit seed split into two i32
};
```

**Browser implementation:** uses `crypto.getRandomValues()` for proper randomness,
with a seedable PCG fallback for `rand_seed()`.

**Node.js test implementation:** uses a seeded PRNG for deterministic tests.

### Time bridge

```js
globalThis.loftHost = {
  // ...
  time_now()                          -> number,   // ms since epoch (like Date.now())
  time_ticks()                        -> number,   // us since program start
};
```

**Node.js:** `Date.now()` for `time_now()`; `process.hrtime.bigint() / 1000n` for
ticks (converted to a number, offset from start).

### Environment bridge

```js
globalThis.loftHost = {
  // ...
  env_variable(name)                  -> string | null,
};
```

**Node.js:** `process.env[name] ?? null`.
**Browser:** returns from a pre-configured `Map` (or null — browsers have no env vars).

### Storage bridge (browser only)

For browser-side persistent storage beyond the virtual filesystem:

```js
globalThis.loftHost = {
  // ...
  storage_get(key)                    -> string | null,
  storage_set(key, value),
  storage_remove(key),
};
```

**Browser:** backed by `localStorage` for small string data or IndexedDB for binary/large
data (see [WEB_IDE.md](WEB_IDE.md) § M4 for IndexedDB project storage).

**Node.js:** backed by a plain `Map` — no persistence needed for tests.

---

## Node.js Test Harness

### Architecture

```
tests/wasm/
├── harness.mjs            — test runner, VirtFS, host setup
├── virt-fs.mjs            — VirtFS class implementation
├── virt-fs.test.mjs       — unit tests for VirtFS itself
├── host.mjs               — loftHost factory for Node.js
├── bridge.test.mjs        — WASM bridge integration tests
├── file-io.test.mjs       — loft file I/O through the virtual FS
├── random.test.mjs        — rand / rand_seed determinism
└── fixtures/
    ├── hello.json          — minimal VirtFS tree
    ├── multi-file.json     — multi-file project tree
    └── binary-data.json    — tree with binary file entries
```

### Running

```sh
# Build WASM for Node.js target
wasm-pack build --target nodejs --out-dir tests/wasm/pkg -- --features wasm

# Run all WASM tests
node --experimental-vm-modules tests/wasm/harness.mjs

# Run a single test file
node --experimental-vm-modules tests/wasm/virt-fs.test.mjs
```

### Host factory (`host.mjs`)

Creates a `loftHost` object wired to a `VirtFS` instance:

```js
import { VirtFS } from './virt-fs.mjs';

export function createHost(tree, options = {}) {
  const fs = new VirtFS(tree);

  // deterministic PRNG (xoshiro128**)
  let rng_state = [1, 2, 3, 4];
  function nextRandom() { /* xoshiro128** body */ }

  const storage = new Map();

  const host = {
    // filesystem — delegates to VirtFS
    fs_exists:       (p) => fs.exists(p),
    fs_read_text:    (p) => fs.readText(p),
    fs_read_binary:  (p, o, n) => fs.readBinary(p)?.slice(o, o + n) ?? null,
    fs_write_text:   (p, c) => { try { fs.writeText(p, c); return 0; } catch { return 5; } },
    fs_write_binary: (p, b) => { try { fs.writeBinary(p, b); return 0; } catch { return 5; } },
    fs_file_size:    (p) => fs.stat(p)?.size ?? -1,
    fs_delete:       (p) => { try { fs.delete(p); return 0; } catch { return 1; } },
    fs_move:         (f, t) => { try { fs.move(f, t); return 0; } catch { return 5; } },
    fs_mkdir:        (p) => { try { fs.mkdir(p); return 0; } catch { return 5; } },
    fs_mkdir_all:    (p) => { try { fs.mkdirAll(p); return 0; } catch { return 5; } },
    fs_list_dir:     (p) => fs.readdir(p) ?? [],
    fs_is_dir:       (p) => fs.isDirectory(p),
    fs_is_file:      (p) => fs.isFile(p),
    fs_seek:         (p, pos) => fs.seek(p, pos),
    fs_read_bytes:   (p, n) => fs.readBytes(p, n),
    fs_write_bytes:  (p, b) => { try { fs.writeBytes(p, b); return 0; } catch { return 5; } },
    fs_get_cursor:   (p) => fs.getCursor(p),
    fs_cwd:          () => fs.cwd,
    fs_user_dir:     () => '/home/test',
    fs_program_dir:  () => '/usr/local/bin',

    // random — deterministic by default
    random_int: (lo, hi) => lo + (nextRandom() % (hi - lo + 1)),
    random_seed: (hi, lo) => { rng_state = [lo, hi, lo ^ hi, lo + hi]; },

    // time
    time_now:   () => options.fakeTime ?? Date.now(),
    time_ticks: () => options.fakeTicks ?? 0,

    // environment
    env_variable: (name) => options.env?.[name] ?? null,

    // storage
    storage_get:    (k) => storage.get(k) ?? null,
    storage_set:    (k, v) => storage.set(k, v),
    storage_remove: (k) => storage.delete(k),
  };

  return { host, fs, storage };
}
```

### VirtFS unit tests (`virt-fs.test.mjs`)

```js
import { test, assert } from './harness.mjs';
import { VirtFS } from './virt-fs.mjs';

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
    "/": {
      "x.txt": { "$type": "text", "$content": "x" },
      "y.txt": { "$type": "text", "$content": "y" },
      "sub": {}
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
```

### WASM bridge integration tests (`bridge.test.mjs`)

These load the actual WASM module with a VirtFS-backed host:

```js
import { test, assert } from './harness.mjs';
import { createHost } from './host.mjs';
import { initWasm, compileAndRun } from './pkg/loft_wasm.js';

const tree = {
  "/": {
    "project": {
      "main.loft": { "$type": "text", "$content": "" }  // overwritten per test
    }
  }
};

let host, fs;

function run(code) {
  ({ host, fs } = createHost(structuredClone(tree)));
  globalThis.loftHost = host;
  fs.writeText('/project/main.loft', code);
  return compileAndRun([{ name: 'main.loft', content: code }]);
}

test('file write and read back', () => {
  const r = run(`
    fn main() {
      f = file("/project/out.txt")
      f.write("hello world")
      g = file("/project/out.txt")
      println(g.content())
    }
  `);
  assert(r.success);
  assert(r.output.trim() === 'hello world');
  assert(fs.readText('/project/out.txt') === 'hello world');
});

test('exists and delete', () => {
  const r = run(`
    fn main() {
      f = file("/project/tmp.txt")
      f.write("x")
      println(exists("/project/tmp.txt"))
      delete("/project/tmp.txt")
      println(exists("/project/tmp.txt"))
    }
  `);
  assert(r.success);
  assert(r.output.trim() === 'true\nfalse');
});

test('directory listing', () => {
  ({ host, fs } = createHost(structuredClone(tree)));
  globalThis.loftHost = host;
  fs.writeText('/project/a.loft', 'fn a() {}');
  fs.writeText('/project/b.loft', 'fn b() {}');

  const r = compileAndRun([{
    name: 'main.loft',
    content: `
      fn main() {
        d = file("/project")
        for f in d.files() { println(f.path) }
      }
    `
  }]);
  assert(r.success);
  assert(r.output.includes('a.loft'));
  assert(r.output.includes('b.loft'));
});

test('rand with seed is deterministic', () => {
  const r1 = run(`
    fn main() {
      rand_seed(42)
      println(rand(1, 1000))
      println(rand(1, 1000))
    }
  `);
  const r2 = run(`
    fn main() {
      rand_seed(42)
      println(rand(1, 1000))
      println(rand(1, 1000))
    }
  `);
  assert(r1.success && r2.success);
  assert(r1.output === r2.output);
});

test('mkdir_all and nested write', () => {
  const r = run(`
    fn main() {
      mkdir_all("/project/a/b/c")
      f = file("/project/a/b/c/deep.txt")
      f.write("nested")
      println(file("/project/a/b/c/deep.txt").content())
    }
  `);
  assert(r.success);
  assert(r.output.trim() === 'nested');
});

test('binary write and read', () => {
  const r = run(`
    fn main() {
      f = file("/project/data.bin")
      f.little_endian()
      f += 42
      f += 256
      g = file("/project/data.bin")
      g.little_endian()
      a = g#read(4) as integer
      b = g#read(4) as integer
      println(a)
      println(b)
    }
  `);
  assert(r.success);
  assert(r.output.trim() === '42\n256');
});
```

### File I/O edge case tests (`file-io.test.mjs`)

```js
test('read nonexistent file returns null content', () => { /* ... */ });
test('write to nested path auto-creates dirs', () => { /* ... */ });
test('delete nonexistent returns NotFound code', () => { /* ... */ });
test('move across directories', () => { /* ... */ });
test('overwrite existing file', () => { /* ... */ });
test('f#size reflects write', () => { /* ... */ });
test('seek beyond end pads with zeros', () => { /* ... */ });
test('binary cursor resets on write', () => { /* ... */ });
```

---

## Browser vs Node.js Host Comparison

| Capability | Browser host | Node.js test host |
|---|---|---|
| Filesystem | VirtFS backed by IndexedDB (persistent) | VirtFS in-memory (ephemeral) |
| Random | `crypto.getRandomValues()` + seedable PCG | Deterministic xoshiro128** |
| Time | `Date.now()` / `performance.now()` | Configurable fake values |
| Environment | Pre-configured `Map` (no real env) | `process.env` or fake `Map` |
| Storage | `localStorage` / IndexedDB | In-memory `Map` |
| Output | Thread-local buffer → console panel | Thread-local buffer → string |
| Binary data | Native `ArrayBuffer` in IndexedDB | base64 in JSON tree |
| Persistence | IndexedDB survives reload | None (per-test lifecycle) |

### When to use which

- **Node.js harness** — CI, automated regression tests, deterministic reproducibility.
  Every test starts from a known JSON tree and gets snapshot isolation.
- **Browser harness** — manual testing, the Web IDE itself. Uses the same `loftHost`
  interface but backed by real browser APIs. Can optionally swap in VirtFS for
  browser-side unit tests too (via `tests/runner.html`).

---

## Implementation Notes

### Rust-side `#[cfg(feature = "wasm")]` dispatch

Each native function that touches the OS gets a conditional branch:

```rust
// Example: file exists check in src/state/io.rs
#[cfg(feature = "wasm")]
fn host_exists(path: &str) -> bool {
    crate::wasm::call_host_bool("fs_exists", path)
}

#[cfg(not(feature = "wasm"))]
fn host_exists(path: &str) -> bool {
    std::path::Path::new(path).exists()
}
```

The `crate::wasm` module provides typed wrappers around the raw `wasm-bindgen` extern
functions declared in `src/wasm.rs`. See [WEB_IDE.md](WEB_IDE.md) § Rust Changes for
the full list of extern declarations.

### Binary data across the WASM boundary

Wasm linear memory and JS `ArrayBuffer` are separate. `wasm-bindgen` handles the
copy automatically for `&[u8]` ↔ `Uint8Array`. The VirtFS stores binary content as
base64 in the JSON tree (for serialisation) but decodes to `Uint8Array` at the API
boundary. In the browser with IndexedDB, binary data stays as `ArrayBuffer` natively
— no base64 overhead.

### Async operations

All `loftHost` functions are **synchronous**. The loft interpreter executes
sequentially and cannot yield to the JS event loop mid-execution. This means:

- **IndexedDB** (async) cannot be called directly from a host function. The browser
  host must pre-load project files into a VirtFS instance before calling
  `compileAndRun()`, and flush writes back to IndexedDB after execution completes.
- **`localStorage`** is synchronous and can be called directly for `storage_*`
  functions.
- For future async needs (HTTP fetch, etc.), the approach is: pre-fetch before
  execution, pass results via the virtual filesystem or a data bridge.

### Error mapping

JS host functions return integer error codes (see the table above). The Rust side
maps these to the `FileResult` enum. If a host function throws a JS exception,
`wasm-bindgen` converts it to a Rust panic — host functions must never throw; they
return error codes instead.

### Test isolation pattern

Every WASM integration test follows this lifecycle:

```
1. createHost(tree)          — fresh VirtFS + host from a fixture
2. globalThis.loftHost = host
3. compileAndRun(code)       — exercises the WASM module
4. assert on output, fs state, storage state
5. (no cleanup needed — next test creates a new host)
```

For tests that mutate the filesystem mid-test and need to verify intermediate states,
use `fs.snapshot()` / `fs.restore()`.

---

## See also
- [WEB_IDE.md](WEB_IDE.md) — Full Web IDE architecture, milestones, Rust changes
- [STDLIB.md](STDLIB.md) § File System — loft file I/O API
- [STDLIB.md](STDLIB.md) § Random — `rand()`, `rand_seed()`, `rand_indices()`
- [INTERNALS.md](INTERNALS.md) — Native function registry and `src/state/io.rs`
- [TESTING.md](TESTING.md) — Rust-side test framework
