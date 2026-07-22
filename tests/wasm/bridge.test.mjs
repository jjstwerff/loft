// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

/**
 * W1.11 — WASM bridge integration tests.
 *
 * Requires the WASM package to be built first:
 *   wasm-pack build --target nodejs --out-dir tests/wasm/pkg -- --no-default-features --features wasm
 *
 * Run:
 *   node tests/wasm/bridge.test.mjs
 */

import { test, assert, run } from './harness.mjs';
import { createHost } from './host.mjs';

// ── Load WASM package (skip gracefully if not built) ──────────────────────────

let compileAndRun;
try {
  ({ compile_and_run: compileAndRun } = await import('./pkg/loft.js'));
} catch {
  console.log('SKIP  bridge tests — WASM package not built');
  console.log('      Run: wasm-pack build --target nodejs --out-dir tests/wasm/pkg -- --no-default-features --features wasm');
  process.exit(0);
}

// ── Test helpers ──────────────────────────────────────────────────────────────

const BASE_TREE = {
  '/': {
    'project': {
      'main.loft': { '$type': 'text', '$content': '' }
    }
  }
};

/**
 * Run a loft program snippet in a fresh VirtFS environment.
 * Returns the parsed `{ output, diagnostics, success }` result object.
 */
function runCode(code) {
  const { host, fs } = createHost(JSON.parse(JSON.stringify(BASE_TREE)));
  globalThis.loftHost = host;
  const raw = compileAndRun(JSON.stringify([{ name: 'main.loft', content: code }]));
  return JSON.parse(raw);
}

// ── Tests ──────────────────────────────────────────────────────────────────────

test('hello world compiles and runs', () => {
  const r = runCode(`fn main() { println("hello") }`);
  assert(r.success, `Expected success; diagnostics: ${r.diagnostics}`);
  assert(r.output.trim() === 'hello');
});

test('file write and read back', () => {
  const { host, fs } = createHost(JSON.parse(JSON.stringify(BASE_TREE)));
  globalThis.loftHost = host;
  const raw = compileAndRun(JSON.stringify([{
    name: 'main.loft',
    content: `
      fn main() {
        f = file("/project/out.txt");
        f.write("hello world");
        println(file("/project/out.txt").content());
      }
    `
  }]));
  const r = JSON.parse(raw);
  assert(r.success, `Expected success; diagnostics: ${r.diagnostics}`);
  assert(r.output.trim() === 'hello world');
  assert(fs.readText('/project/out.txt') === 'hello world');
});

test('exists and delete', () => {
  const r = runCode(`
    fn main() {
      f = file("/project/tmp.txt");
      f.write("x");
      b1 = exists("/project/tmp.txt");
      println("{b1}");
      delete("/project/tmp.txt");
      b2 = exists("/project/tmp.txt");
      println("{b2}");
    }
  `);
  assert(r.success, `Expected success; diagnostics: ${r.diagnostics}`);
  assert(r.output.trim() === 'true\nfalse');
});

test('directory listing', () => {
  const { host, fs } = createHost(JSON.parse(JSON.stringify(BASE_TREE)));
  globalThis.loftHost = host;
  fs.writeText('/project/a.loft', 'fn a() {}');
  fs.writeText('/project/b.loft', 'fn b() {}');
  const raw = compileAndRun(JSON.stringify([{
    name: 'main.loft',
    content: `
      fn main() {
        d = file("/project");
        for f in d.files() { println(f.path); }
      }
    `
  }]));
  const r = JSON.parse(raw);
  assert(r.success, `Expected success; diagnostics: ${r.diagnostics}`);
  assert(r.output.includes('a.loft'));
  assert(r.output.includes('b.loft'));
});

// NOTE: the seeded-`rand` determinism test was removed here. `rand` / `rand_seed`
// are no longer core loft builtins — they were drained to the `random` library
// (@PLAN12 phase 3.5a; see src/codegen_runtime.rs), so `Unknown function rand_seed`
// even in a default build. The core `--features wasm` bundle these bridge tests
// drive does not carry the `random` lib, so this coverage belongs in a
// library-level test, not the core bridge suite.

test('mkdir_all and nested write', () => {
  const r = runCode(`
    fn main() {
      mkdir_all("/project/a/b/c");
      f = file("/project/a/b/c/deep.txt");
      f.write("nested");
      println(file("/project/a/b/c/deep.txt").content());
    }
  `);
  assert(r.success, `Expected success; diagnostics: ${r.diagnostics}`);
  assert(r.output.trim() === 'nested');
});

test('compile error is reported', () => {
  const r = runCode(`fn main() { this is not valid loft syntax %%%`);
  assert(!r.success, 'Expected failure for invalid syntax');
  assert(typeof r.diagnostics === 'string' && r.diagnostics.length > 0);
});

// W1.16 — binary file I/O end-to-end
test('binary write and read back (BigEndian)', () => {
  const r = runCode(`
    fn main() {
     {f = file("/project/data.bin");
      f#format = BigEndian;
      f += 0x01020304 as i32;
     }
     {f = file("/project/data.bin");
      f#format = BigEndian;
      v = f#read(4) as i32;
      println("{v}");
     }
    }
  `);
  assert(r.success, `Expected success; diagnostics: ${r.diagnostics}`);
  // BigEndian 0x01020304 read back as BigEndian i32 = 16909060
  assert(r.output.trim() === '16909060', `Got: ${r.output.trim()}`);
});

test('binary seek and partial read', () => {
  const r = runCode(`
    fn main() {
     {f = file("/project/seek.bin");
      f#format = LittleEndian;
      f += 10 as i32;
      f += 20 as i32;
      f += 30 as i32;
     }
     {f = file("/project/seek.bin");
      f#format = LittleEndian;
      f#next = 4;
      v = f#read(4) as i32;
      println("{v}");
      n = f#next;
      println("{n}");
     }
    }
  `);
  assert(r.success, `Expected success; diagnostics: ${r.diagnostics}`);
  const lines = r.output.trim().split('\n');
  assert(lines[0] === '20', `Expected 20, got ${lines[0]}`);
  assert(lines[1] === '8', `Expected next=8, got ${lines[1]}`);
});

test('truncate file with f#size', () => {
  const r = runCode(`
    fn main() {
     {f = file("/project/trunc.bin");
      f#format = LittleEndian;
      f += 1 as i32;
      f += 2 as i32;
      f += 3 as i32;
      f += 4 as i32;
     }
     {f = file("/project/trunc.bin");
      f#format = LittleEndian;
      f#size = 8;
      sz = f#size;
      println("{sz}");
     }
    }
  `);
  assert(r.success, `Expected success; diagnostics: ${r.diagnostics}`);
  assert(r.output.trim() === '8', `Got: ${r.output.trim()}`);
});

test('@PLN13 — a beginner script (no fn main, no semicolons) runs in the playground', () => {
  // Loose top-level statements, no `fn main`, `;` omitted between them: the wasm
  // entry point auto-detects the script and desugars it, exactly like the CLI.
  const r = runCode(`
total = 0
for i in 1..5 {
  total = total + i;
}
println("total={total}")
`);
  assert(r.success, `Expected success; diagnostics: ${r.diagnostics}`);
  assert(r.output.trim() === 'total=10', `Got: ${r.output.trim()}`);
});

// ── Run ────────────────────────────────────────────────────────────────────────

const failed = await run();
process.exit(failed > 0 ? 1 : 0);
