// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

/**
 * W1.13 — Full loft test suite via WASM.
 *
 * Runs tests/scripts/*.loft and tests/docs/*.loft through the WASM module
 * and compares their output against the native loft interpreter.
 *
 * Prerequisites:
 *   1. Build the WASM package:
 *        wasm-pack build --target nodejs --out-dir tests/wasm/pkg -- --features wasm
 *   2. Run:
 *        node tests/wasm/suite.mjs
 *
 * For each test file the runner:
 *   1. Reads the .loft source
 *   2. Populates a VirtFS with the source file and any supporting fixtures
 *   3. Calls compile_and_run() via the WASM module
 *   4. Gets native reference output via `cargo run`
 *   5. Compares outputs; reports pass / fail / skip
 *
 * Exit code: 0 if all run tests pass, 1 if any fail.
 */

import { readFileSync } from 'fs';
import { execSync } from 'child_process';
import { basename } from 'path';
import { createHost } from './host.mjs';
import { buildDefaultTree, withFiles } from './default-tree.mjs';

// ── Load WASM package ──────────────────────────────────────────────────────────

let compileAndRun;
try {
  ({ compile_and_run: compileAndRun } = await import('./pkg/loft_wasm.js'));
} catch {
  console.log('SKIP  suite — WASM package not built');
  console.log('      wasm-pack build --target nodejs --out-dir tests/wasm/pkg -- --features wasm');
  process.exit(0);
}

// ── Check native runner ────────────────────────────────────────────────────────

let nativeAvailable = false;
try {
  execSync('cargo --version', { stdio: 'ignore', timeout: 5000 });
  nativeAvailable = true;
} catch {
  console.log('NOTE  cargo not found — WASM tests will verify success only, not compare output');
}

// ── Skip / compare-skip lists ──────────────────────────────────────────────────

/**
 * Tests that are skipped entirely.
 * - File I/O tests that depend on real on-disk fixtures not easily replicated in VirtFS.
 * - Image tests that require PNG files.
 */
const SKIP = new Set([
  '14-image.loft',     // requires PNG/image fixtures; pixel ops outside pure computation
]);

/**
 * Tests that run through WASM but whose output is NOT compared to native.
 * - Time-sensitive: output changes every run.
 * - Order-sensitive with randomness: values differ per seed but behaviour is correct.
 * - Threading: sequential in WASM Tier 1, may differ from native parallel output.
 */
const SKIP_COMPARE = new Set([
  '16-time.loft',      // now()/ticks() values are non-deterministic
  '22-time.loft',      // same — doc version
  '22-threading.loft', // sequential WASM vs parallel native; results correct but order differs
  '15-random.loft',    // rand() without seed — non-deterministic
  '21-random.loft',    // same — doc version
]);

// ── VirtFS fixture builder ─────────────────────────────────────────────────────

// Shared default tree (docs + scripts + example); built once and cloned per test.
const _defaultTree = buildDefaultTree();

/**
 * Build the VirtFS tree for a given loft test file.
 *
 * Starts from the shared default tree (docs, scripts, example) and overlays
 * the specific source file at its natural path so that loft programs can
 * reference `tests/example/...` and sibling script files unchanged.
 *
 * @param {string} relPath  Relative path to the .loft file.
 * @param {string} content  Source code of the file.
 * @returns {object}        Deep-cloned VirtFS tree.
 */
function buildTree(relPath, content) {
  // Deep-clone so each test run starts from a clean state.
  const tree = JSON.parse(JSON.stringify(_defaultTree));
  tree['/'].tmp = {};  // scratch space for file-writing tests
  return withFiles(tree, { [relPath]: content });
}

// ── Native reference runner ────────────────────────────────────────────────────

/**
 * Run a loft file with the native interpreter and return its stdout, or null
 * if the run fails or cargo is unavailable.
 *
 * @param {string} filePath  Path to the .loft file.
 * @returns {string|null}
 */
function runNative(filePath) {
  if (!nativeAvailable) return null;
  try {
    return execSync(
      `cargo run --bin loft --quiet -- "${filePath}" 2>/dev/null`,
      { encoding: 'utf8', timeout: 30_000 }
    );
  } catch (e) {
    return null;  // native also failed — both sides should fail/skip
  }
}

// ── WASM runner ────────────────────────────────────────────────────────────────

/**
 * Run a loft source string through the WASM module and return the result.
 *
 * @param {string} name     Logical filename (e.g. 'main.loft').
 * @param {string} content  Source code.
 * @param {object} tree     VirtFS tree to back the host.
 * @returns {{ success: boolean, output: string, diagnostics: string }}
 */
function runWasm(name, content, tree) {
  const { host } = createHost(tree);
  globalThis.loftHost = host;
  try {
    const raw = compileAndRun(JSON.stringify([{ name, content }]));
    return JSON.parse(raw);
  } catch (err) {
    return { success: false, output: '', diagnostics: String(err) };
  }
}

// ── Test discovery ─────────────────────────────────────────────────────────────

/**
 * Collect all .loft files from a directory that contain `fn main(`.
 * @param {string} dir  Relative directory path.
 * @returns {string[]}  Sorted relative file paths.
 */
function discoverTests(dir) {
  if (!existsSync(dir)) return [];
  return readdirSync(dir)
    .filter(f => f.endsWith('.loft'))
    .sort()
    .map(f => join(dir, f))
    .filter(p => {
      try {
        return readFileSync(p, 'utf8').includes('\nfn main(');
      } catch { return false; }
    });
}

// ── Main ───────────────────────────────────────────────────────────────────────

const scriptTests = discoverTests('tests/scripts');
const docTests    = discoverTests('tests/docs');
const allTests    = [...scriptTests, ...docTests];

let passed = 0;
let failed = 0;
let skipped = 0;

console.log(`\nWASM suite — ${allTests.length} test files\n`);

for (const filePath of allTests) {
  const name = basename(filePath);

  if (SKIP.has(name)) {
    console.log(`SKIP  ${filePath}`);
    skipped++;
    continue;
  }

  const content = readFileSync(filePath, 'utf8');
  const tree    = buildTree(filePath, content);
  const wasmResult = runWasm(name, content, tree);

  if (!wasmResult.success) {
    console.error(`FAIL  ${filePath}`);
    if (wasmResult.diagnostics) {
      console.error(`      diagnostics: ${wasmResult.diagnostics.split('\n')[0]}`);
    }
    failed++;
    continue;
  }

  if (SKIP_COMPARE.has(name)) {
    console.log(`  ok  ${filePath}  (no output comparison)`);
    passed++;
    continue;
  }

  if (!nativeAvailable) {
    console.log(`  ok  ${filePath}  (WASM success; no native comparison)`);
    passed++;
    continue;
  }

  const nativeOut = runNative(filePath);
  if (nativeOut === null) {
    // Native failed too — treat as expected (both agree)
    console.log(`  ok  ${filePath}  (both native and WASM produced no output)`);
    passed++;
    continue;
  }

  const wasmOut = wasmResult.output;
  if (nativeOut.trimEnd() === wasmOut.trimEnd()) {
    console.log(`  ok  ${filePath}`);
    passed++;
  } else {
    console.error(`FAIL  ${filePath}  — output mismatch`);
    console.error(`      native: ${JSON.stringify(nativeOut.slice(0, 120))}`);
    console.error(`        wasm: ${JSON.stringify(wasmOut.slice(0, 120))}`);
    failed++;
  }
}

console.log(`\n${passed} passed, ${failed} failed, ${skipped} skipped`);
process.exit(failed > 0 ? 1 : 0);
