// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

/**
 * W1.11 — WASM bridge integration tests.
 *
 * Requires the WASM package to be built first:
 *   wasm-pack build --target nodejs --out-dir tests/wasm/pkg -- --features wasm
 *
 * Run:
 *   node tests/wasm/bridge.test.mjs
 */

import { test, assert, run } from './harness.mjs';
import { createHost } from './host.mjs';

// ── Load WASM package (skip gracefully if not built) ──────────────────────────

let compileAndRun;
try {
  ({ compile_and_run: compileAndRun } = await import('./pkg/loft_wasm.js'));
} catch {
  console.log('SKIP  bridge tests — WASM package not built');
  console.log('      Run: wasm-pack build --target nodejs --out-dir tests/wasm/pkg -- --features wasm');
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
        f = file("/project/out.txt")
        f.write("hello world")
        g = file("/project/out.txt")
        println(g.content())
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
      f = file("/project/tmp.txt")
      f.write("x")
      println(exists("/project/tmp.txt"))
      delete("/project/tmp.txt")
      println(exists("/project/tmp.txt"))
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
        d = file("/project")
        for f in d.files() { println(f.path) }
      }
    `
  }]));
  const r = JSON.parse(raw);
  assert(r.success, `Expected success; diagnostics: ${r.diagnostics}`);
  assert(r.output.includes('a.loft'));
  assert(r.output.includes('b.loft'));
});

test('rand with seed is deterministic', () => {
  const code = `
    fn main() {
      rand_seed(42)
      println(rand(1, 1000))
      println(rand(1, 1000))
    }
  `;
  const r1 = runCode(code);
  const r2 = runCode(code);
  assert(r1.success && r2.success, 'Expected both runs to succeed');
  assert(r1.output === r2.output, 'Expected same output from seeded rand');
});

test('mkdir_all and nested write', () => {
  const r = runCode(`
    fn main() {
      mkdir_all("/project/a/b/c")
      f = file("/project/a/b/c/deep.txt")
      f.write("nested")
      println(file("/project/a/b/c/deep.txt").content())
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

// ── Run ────────────────────────────────────────────────────────────────────────

const failed = await run();
process.exit(failed > 0 ? 1 : 0);
