// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

/**
 * @PLN13 phase 5 — the BROWSER half of script mode.
 *
 * `tests/script_mode.rs` proves a script runs on the interpreter and on
 * `--native`.  Neither of those touches the path the playground actually uses:
 * `compile_and_run` in `src/wasm.rs`, which applies `script::script_desugar`
 * before compiling.  That call is the whole acquisition funnel — a newcomer
 * types two lines on a web page and sees output — and nothing guarded it, so
 * dropping the desugar would have broken the playground with every Rust test
 * still green.
 *
 * Run:
 *   node tests/wasm/script.test.mjs
 *
 * Requires the nodejs bundle (the `wasm-bridge` CI job builds it):
 *   wasm-pack build --target nodejs --out-dir tests/wasm/pkg -- --no-default-features --features wasm
 */

import { test, assert, run } from './harness.mjs';
import { createHost } from './host.mjs';

// ── Load WASM package ─────────────────────────────────────────────────────────
//
// Missing bundle is a SKIP locally (a dev without wasm-pack still gets a useful
// run) but a FAILURE under CI, where the job builds it first: there, "skipped"
// can only mean the build step broke, and a guard that skips itself green is
// exactly the rot this file exists to prevent.

let compileAndRun;
try {
  ({ compile_and_run: compileAndRun } = await import('./pkg/loft.js'));
} catch (e) {
  if (process.env.CI) {
    console.error('FAIL  script tests — WASM package not built, but CI builds it before this step');
    console.error(`      ${e && e.message}`);
    process.exit(1);
  }
  console.log('SKIP  script tests — WASM package not built');
  console.log('      Run: wasm-pack build --target nodejs --out-dir tests/wasm/pkg -- --no-default-features --features wasm');
  process.exit(0);
}

const BASE_TREE = { '/': { 'project': { 'main.loft': { '$type': 'text', '$content': '' } } } };

/** Compile+run `code` exactly as the playground does, returning `{output, success}`. */
function runCode(code) {
  const { host } = createHost(JSON.parse(JSON.stringify(BASE_TREE)));
  globalThis.loftHost = host;
  return JSON.parse(compileAndRun(JSON.stringify([{ name: 'main.loft', content: code }])));
}

// ── Tests ─────────────────────────────────────────────────────────────────────

// The playground's own default sample shape: no `fn main`, no semicolons, state
// shared across loose statements in source order.
test('a bare script runs in the browser bundle', () => {
  const r = runCode('name = "world"\nprint("hello {name}\\n")\n');
  assert(r.success, `script should run: ${r.diagnostics}`);
  assert(r.output === 'hello world\n', `expected "hello world\\n", got ${JSON.stringify(r.output)}`);
});

// Statements run ONCE, in order — a script desugars to a run-once `fn main`,
// not to the REPL's re-accumulate.
test('loose statements run once, in source order', () => {
  const r = runCode('n = 2\nn = n + 3\nprint("{n}\\n")\n');
  assert(r.success, `script should run: ${r.diagnostics}`);
  assert(r.output === '5\n', `expected "5\\n", got ${JSON.stringify(r.output)}`);
});

// A top-level def stays top-level and callable from the loose statements.
test('a script may define and call its own function', () => {
  const r = runCode('fn double(x: integer) -> integer { x * 2 }\nprint("{double(21)}\\n")\n');
  assert(r.success, `script should run: ${r.diagnostics}`);
  assert(r.output === '42\n', `expected "42\\n", got ${JSON.stringify(r.output)}`);
});

// The other half of the contract: a conventional program is NOT a script, so
// the desugar must leave it alone rather than wrap it a second time.
test('a conventional `fn main` program is untouched', () => {
  const r = runCode('fn main() {\n  print("classic\\n");\n}\n');
  assert(r.success, `program should run: ${r.diagnostics}`);
  assert(r.output === 'classic\n', `expected "classic\\n", got ${JSON.stringify(r.output)}`);
});

await run();
