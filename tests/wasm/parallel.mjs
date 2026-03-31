// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//
// W1.18-4: Thread pool manager for WASM parallel execution.
// Spawns N Worker Threads, each running worker.mjs with a shared WASM module
// and SharedArrayBuffer-backed memory.  Distributes par() loops by writing
// work parameters to a control buffer and signalling workers via Atomics.

import { Worker } from 'node:worker_threads';
import { fileURLToPath } from 'node:url';

const WORKER_SCRIPT = fileURLToPath(new URL('./worker.mjs', import.meta.url));

/**
 * Thread pool for WASM parallel execution.
 *
 * @example
 *   const pool = new LoftThreadPool(module, memory, 4);
 *   await pool.waitReady();
 *   pool.runParallel(fnIndex, totalElements);
 *   await pool.terminate();
 */
export class LoftThreadPool {
  /**
   * @param {WebAssembly.Module} module  — compiled WASM module (structured-cloneable)
   * @param {WebAssembly.Memory} memory  — shared memory (SharedArrayBuffer-backed)
   * @param {number} nWorkers            — number of worker threads
   */
  constructor(module, memory, nWorkers) {
    this.nWorkers = nWorkers;
    // Control buffer: 4 slots per worker (command, fn_index, start, end).
    this.control = new Int32Array(
      new SharedArrayBuffer(4 * nWorkers * 4)
    );
    // Done signal: 1 slot per worker.
    this.done = new Int32Array(new SharedArrayBuffer(nWorkers * 4));

    this.workers = Array.from({ length: nWorkers }, (_, id) =>
      new Worker(WORKER_SCRIPT, {
        workerData: {
          module,
          memory,
          control: this.control,
          done: this.done,
          workerId: id,
        },
      })
    );
  }

  /** Wait for all workers to post { type: 'ready' }. */
  async waitReady() {
    await Promise.all(
      this.workers.map(
        (w) =>
          new Promise((resolve) => {
            w.once('message', (msg) => {
              if (msg.type === 'ready') resolve();
            });
          })
      )
    );
  }

  /**
   * Distribute a par() loop across all workers and wait for completion.
   * @param {number} fnIndex  — WASM function table index for the worker body
   * @param {number} total    — total number of elements
   */
  runParallel(fnIndex, total) {
    const N = this.nWorkers;
    const chunkSize = Math.ceil(total / N);

    for (let t = 0; t < N; t++) {
      const start = t * chunkSize;
      const end = Math.min(start + chunkSize, total);
      Atomics.store(this.done, t, 0);
      Atomics.store(this.control, N + t, fnIndex);
      Atomics.store(this.control, N * 2 + t, start);
      Atomics.store(this.control, N * 3 + t, end);
      Atomics.store(this.control, t, 1); // command = run
      Atomics.notify(this.control, t);
    }

    // Main thread waits for all workers to complete.
    for (let t = 0; t < N; t++) {
      Atomics.wait(this.done, t, 0); // block until done[t] != 0
    }
  }

  /** Shut down all workers. */
  async terminate() {
    for (let t = 0; t < this.nWorkers; t++) {
      Atomics.store(this.control, t, 2); // command = exit
      Atomics.notify(this.control, t);
    }
    await Promise.all(this.workers.map((w) => w.terminate()));
  }
}
