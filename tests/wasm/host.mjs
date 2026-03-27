// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

/**
 * W1.11 — loftHost factory for Node.js WASM tests.
 *
 * Creates a `loftHost` object wired to a `VirtFS` instance, ready to be
 * assigned to `globalThis.loftHost` before calling `compileAndRun()`.
 *
 * Usage:
 *   const { host, fs, storage } = createHost(tree, options);
 *   globalThis.loftHost = host;
 *
 * Options:
 *   fakeTime   — override `time_now()` return value (ms since epoch)
 *   fakeTicks  — override `time_ticks()` return value (µs)
 *   env        — plain object used for `env_variable(name)` lookups
 *   args       — string[] returned by `arguments()`
 */

'use strict';

import { VirtFS } from './virt-fs.mjs';

// ── xoshiro128** PRNG ──────────────────────────────────────────────────────────
// Reference: https://prng.di.unimi.it/xoshiro128starstar.c
// All arithmetic is unsigned 32-bit.

function _rotl(x, k) {
  return ((x << k) | (x >>> (32 - k))) >>> 0;
}

function _makeRng(seed = [1, 2, 3, 4]) {
  // State: four uint32 values (stored as regular JS numbers, masked to 32-bit).
  let [s0, s1, s2, s3] = seed.map(v => v >>> 0);

  return {
    next() {
      const result = Math.imul(_rotl(Math.imul(s1, 5) >>> 0, 7), 9) >>> 0;
      const t = (s1 << 9) >>> 0;
      s2 = (s2 ^ s0) >>> 0;
      s3 = (s3 ^ s1) >>> 0;
      s1 = (s1 ^ s2) >>> 0;
      s0 = (s0 ^ s3) >>> 0;
      s2 = (s2 ^ t) >>> 0;
      s3 = _rotl(s3, 11);
      return result;
    },
    seed(hi, lo) {
      s0 = lo >>> 0;
      s1 = hi >>> 0;
      s2 = (lo ^ hi) >>> 0;
      s3 = (lo + hi) >>> 0;
    },
  };
}

// ── createHost ─────────────────────────────────────────────────────────────────

/**
 * Create a loftHost + VirtFS pair for use in Node.js WASM tests.
 *
 * @param {object} tree      Initial VirtFS tree (default: empty root).
 * @param {object} options   Optional overrides (fakeTime, fakeTicks, env, args).
 * @returns {{ host: object, fs: VirtFS, storage: Map }}
 */
export function createHost(tree = { '/': {} }, options = {}) {
  const fs = new VirtFS(tree);
  const rng = _makeRng();
  const storage = new Map();

  const host = {
    // ── filesystem — delegates to VirtFS ──────────────────────────────────────
    fs_exists:       (p) => fs.exists(p),
    fs_read_text:    (p) => fs.readText(p),
    fs_read_binary:  (p, o, n) => {
      const all = fs.readBinary(p);
      return all ? all.slice(o, o + n) : null;
    },
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
    fs_seek:         (p, pos) => { fs.seek(p, pos); },
    fs_read_bytes:   (p, n) => fs.readBytes(p, n),
    fs_write_bytes:  (p, b) => { try { fs.writeBytes(p, b); return 0; } catch { return 5; } },
    fs_get_cursor:   (p) => fs.getCursor(p),
    fs_cwd:          () => fs.cwd,
    fs_user_dir:     () => '/home/test',
    fs_program_dir:  () => '/usr/local/bin',

    // ── random — deterministic xoshiro128** ───────────────────────────────────
    // `random_int(lo, hi)` returns an integer in the closed interval [lo, hi].
    random_int:  (lo, hi) => {
      const range = (hi - lo + 1) >>> 0;
      return lo + (rng.next() % range);
    },
    // The Rust side passes a 64-bit seed split as (seed_hi: i32, seed_lo: i32).
    random_seed: (hi, lo) => rng.seed(hi, lo),

    // ── time ──────────────────────────────────────────────────────────────────
    time_now:   () => options.fakeTime  ?? Date.now(),
    time_ticks: () => options.fakeTicks ?? 0,

    // ── environment ───────────────────────────────────────────────────────────
    env_variable: (name) => options.env?.[name] ?? null,

    // ── arguments ─────────────────────────────────────────────────────────────
    arguments: () => options.args ?? [],

    // ── logging — delegates to console ────────────────────────────────────────
    log_write: (level, msg) => {
      const fn_ = level === 'fatal' ? 'error' : level;
      (console[fn_] ?? console.log)(`[loft] ${msg}`);
    },

    // ── storage — in-memory Map ───────────────────────────────────────────────
    storage_get:    (k) => storage.get(k) ?? null,
    storage_set:    (k, v) => { storage.set(k, v); },
    storage_remove: (k) => { storage.delete(k); },
  };

  return { host, fs, storage };
}
