// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

/**
 * W1.11 — rand / rand_seed determinism tests.
 *
 * Host-level: verify the xoshiro128** PRNG in host.mjs is deterministic and
 * produces values in the correct range.  No WASM needed.
 *
 * The WASM-level `rand()` / `rand_seed()` layer this file used to carry is gone —
 * see the note further down.
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

// ── WASM-level `rand` / `rand_seed` tests: REMOVED, not skipped ───────────────
//
// `rand` / `rand_seed` moved OUT of the core bundle into the `random` library
// (@PLAN12), so testing them against a `--features wasm` build asserted a
// capability that is deliberately gone: the three cases failed with "Unknown
// function rand", and because this file is not part of any CI job nobody saw it.
// Their coverage belongs to the `random` library's own tests. What stays here is
// the layer that IS still core — the host-side xoshiro128** PRNG above.

// ── Run ────────────────────────────────────────────────────────────────────────

const failed = await run();
process.exit(failed > 0 ? 1 : 0);
