// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//
// W1.18-3: Worker thread entry point for WASM parallel execution.
// Each worker receives a shared WASM module + memory via workerData,
// instantiates it, then parks in a loop waiting for commands via
// a SharedArrayBuffer control signal.

import { parentPort, workerData } from 'node:worker_threads';

const { module, memory, control, done, workerId } = workerData;

// Instantiate the WASM module with shared memory.
// The host bridge is minimal for workers — no file I/O, just computation.
const importObject = {
  env: { memory },
  loftHost: {
    // Workers don't need file I/O — they only compute.
    // Add minimal stubs so the module instantiates without errors.
    print: (ptr, len) => {},
    println: (ptr, len) => {},
  },
};

const { instance } = await WebAssembly.instantiate(module, importObject);
const { worker_entry } = instance.exports;

// Signal ready to the main thread.
Atomics.store(done, workerId, 0);
parentPort.postMessage({ type: 'ready' });

// Work loop — park until the main thread signals a command.
const N = done.length;
while (true) {
  Atomics.wait(control, workerId, 0); // sleep until control[workerId] != 0
  const cmd = Atomics.load(control, workerId);

  if (cmd === 2) break; // exit command

  // Read work parameters from the control buffer.
  const fnIndex = Atomics.load(control, N + workerId);
  const start = Atomics.load(control, N * 2 + workerId);
  const end = Atomics.load(control, N * 3 + workerId);

  // Execute the worker function for elements [start, end).
  worker_entry(fnIndex, start, end);

  // Signal completion.
  Atomics.store(done, workerId, 1);
  Atomics.notify(done, workerId);
  Atomics.store(control, workerId, 0); // reset to idle
}
