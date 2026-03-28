// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

/**
 * W1.11 — rand / rand_seed determinism tests.
 *
 * Two layers:
 *   1. Host-level: verify the xoshiro128** PRNG in host.mjs is deterministic
 *      and produces values in the correct range.  No WASM needed.
 *   2. WASM-level: verify that loft's `rand()` / `rand_seed()` produce the
 *      same output when seeded identically.  Requires the WASM package.
 *
 * Run:
 *   node tests/wasm/random.test.mjs
 */

import { test, assert, run } from './harness.mjs';
import { createHost } from './host.mjs';

// ── Host-level PRNG tests (no WASM required) ──────────────────────────────────

test('random_int stays within bounds', () => {
  const { host } = createHost({ '/': {} });
  for (let i = 0; i < 1000; i++) {
    const v = host.random_int(1, 10);
    assert(v >= 1 && v <= 10, `Out of range: ${v}`);
  }
});

test('random_int with same seed produces same sequence', () => {
  const { host: h1 } = createHost({ '/': {} });
  const { host: h2 } = createHost({ '/': {} });

  h1.random_seed(0, 42);
  h2.random_seed(0, 42);

  for (let i = 0; i < 20; i++) {
    const v1 = h1.random_int(0, 1000000);
    const v2 = h2.random_int(0, 1000000);
    assert(v1 === v2, `Diverged at step ${i}: ${v1} vs ${v2}`);
  }
});

test('random_int without seed is not trivially constant', () => {
  const { host } = createHost({ '/': {} });
  // Default seeds [1,2,3,4] — just verify the sequence is not all the same value.
  const seen = new Set();
  for (let i = 0; i < 20; i++) seen.add(host.random_int(0, 1000000));
  assert(seen.size > 1, 'PRNG appears stuck — all values identical');
});

test('different seeds produce different sequences', () => {
  const { host: h1 } = createHost({ '/': {} });
  const { host: h2 } = createHost({ '/': {} });

  h1.random_seed(0, 1);
  h2.random_seed(0, 99999);

  let differ = false;
  for (let i = 0; i < 20; i++) {
    if (h1.random_int(0, 1000000) !== h2.random_int(0, 1000000)) { differ = true; break; }
  }
  assert(differ, 'Different seeds produced identical sequences');
});

test('random_int(n, n) always returns n', () => {
  const { host } = createHost({ '/': {} });
  for (let i = 0; i < 10; i++) {
    assert(host.random_int(7, 7) === 7);
  }
});

// ── WASM-level tests (require wasm-pack build) ────────────────────────────────

let compileAndRun;
try {
  ({ compile_and_run: compileAndRun } = await import('./pkg/loft.js'));
} catch {
  console.log('NOTE  WASM-level random tests skipped — package not built');
  console.log('      Run: wasm-pack build --target nodejs --out-dir tests/wasm/pkg -- --no-default-features --features wasm');
}

if (compileAndRun) {
  const BASE_TREE = { '/': { 'project': { 'main.loft': { '$type': 'text', '$content': '' } } } };

  function runCode(code) {
    const { host } = createHost(JSON.parse(JSON.stringify(BASE_TREE)));
    globalThis.loftHost = host;
    const raw = compileAndRun(JSON.stringify([{ name: 'main.loft', content: code }]));
    return JSON.parse(raw);
  }

  test('loft rand_seed produces deterministic output', () => {
    const code = `
      fn main() {
        rand_seed(42)
        println(rand(1, 1000))
        println(rand(1, 1000))
        println(rand(1, 1000))
      }
    `;
    const r1 = runCode(code);
    const r2 = runCode(code);
    assert(r1.success, `Run 1 failed: ${r1.diagnostics}`);
    assert(r2.success, `Run 2 failed: ${r2.diagnostics}`);
    assert(r1.output === r2.output, `Output differed:\n  run1: ${r1.output}\n  run2: ${r2.output}`);
  });

  test('loft rand_seed(0) != rand_seed(1) output', () => {
    const seed0 = runCode(`fn main() { rand_seed(0)\nprintln(rand(1, 1000000)) }`);
    const seed1 = runCode(`fn main() { rand_seed(1)\nprintln(rand(1, 1000000)) }`);
    assert(seed0.success && seed1.success);
    // Different seeds should (with overwhelming probability) produce different first values.
    assert(seed0.output !== seed1.output, 'Expected different output for different seeds');
  });

  test('loft rand without seed succeeds', () => {
    const r = runCode(`fn main() { println(rand(1, 100)) }`);
    assert(r.success, `Expected success; diagnostics: ${r.diagnostics}`);
    const v = parseInt(r.output.trim(), 10);
    assert(v >= 1 && v <= 100, `Out of range: ${v}`);
  });
}

// ── Run ────────────────────────────────────────────────────────────────────────

const failed = await run();
process.exit(failed > 0 ? 1 : 0);
