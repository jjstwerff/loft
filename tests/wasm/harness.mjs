// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

/**
 * W1.10 — Minimal test runner for the WASM test suite.
 *
 * Provides `test(name, fn)`, `assert(cond)`, `assert.deepEqual(a, b)`,
 * and `assert.throws(fn)`.  Designed to run under:
 *
 *   node tests/wasm/harness.mjs              (runs all registered tests)
 *   node tests/wasm/virt-fs.test.mjs         (runs VirtFS tests directly)
 *
 * No external dependencies.  Exit code: 0 on success, 1 on any failure.
 */

'use strict';

// ── Test registry ─────────────────────────────────────────────────────────────

const _tests = [];

/**
 * Register a named test case.
 * @param {string} name
 * @param {() => void | Promise<void>} fn
 */
export function test(name, fn) {
  _tests.push({ name, fn });
}

// ── Assertion helpers ─────────────────────────────────────────────────────────

/**
 * Assert a truthy condition.
 * @param {*} cond
 * @param {string} [message]
 */
export function assert(cond, message) {
  if (!cond) {
    throw new Error(message ?? `Assertion failed: expected truthy, got ${JSON.stringify(cond)}`);
  }
}

/**
 * Deep-equality assertion for primitives, arrays, and plain objects.
 * Also handles Uint8Array comparison.
 */
assert.deepEqual = function deepEqual(actual, expected, message) {
  if (!_deepEq(actual, expected)) {
    throw new Error(
      message ??
        `deepEqual failed:\n  actual:   ${_fmt(actual)}\n  expected: ${_fmt(expected)}`
    );
  }
};

/**
 * Assert that `fn` throws an error (any error).
 */
assert.throws = function throws(fn, message) {
  let threw = false;
  try {
    fn();
  } catch (_) {
    threw = true;
  }
  if (!threw) {
    throw new Error(message ?? 'Expected function to throw but it did not');
  }
};

// ── Internal helpers ──────────────────────────────────────────────────────────

function _fmt(v) {
  if (v instanceof Uint8Array) return `Uint8Array([${v.join(', ')}])`;
  try { return JSON.stringify(v); } catch { return String(v); }
}

function _deepEq(a, b) {
  if (a === b) return true;
  if (a instanceof Uint8Array && b instanceof Uint8Array) {
    if (a.length !== b.length) return false;
    for (let i = 0; i < a.length; i++) if (a[i] !== b[i]) return false;
    return true;
  }
  if (Array.isArray(a) && Array.isArray(b)) {
    if (a.length !== b.length) return false;
    return a.every((v, i) => _deepEq(v, b[i]));
  }
  if (a !== null && b !== null && typeof a === 'object' && typeof b === 'object') {
    const ka = Object.keys(a).sort();
    const kb = Object.keys(b).sort();
    if (!_deepEq(ka, kb)) return false;
    return ka.every(k => _deepEq(a[k], b[k]));
  }
  return false;
}

// ── Runner ────────────────────────────────────────────────────────────────────

/**
 * Run all registered tests and print results to stdout.
 * Returns the number of failures.
 */
export async function runAll() {
  let passed = 0;
  let failed = 0;
  for (const { name, fn } of _tests) {
    try {
      await fn();
      console.log(`  ok  ${name}`);
      passed++;
    } catch (err) {
      console.error(`FAIL  ${name}`);
      console.error(`      ${err.message}`);
      failed++;
    }
  }
  console.log(`\n${passed} passed, ${failed} failed`);
  return failed;
}

// ── Auto-run when this file is the entry point ────────────────────────────────
// When a test file imports harness.mjs and is run directly, the test file
// registers its tests via `test(...)` at import time.  We detect the entry
// point and call runAll() after all synchronous imports have settled.

// `import.meta.url` comparison detects whether this module is the entry point.
// For test files that run directly (e.g. `node virt-fs.test.mjs`), the test
// file itself is the entry point — not harness.mjs.  So harness.mjs does NOT
// auto-run here; instead each test file calls `runAll()` at the end.
// This export gives test files a convenient way to do so:

export { runAll as run };
