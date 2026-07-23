// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//
// @PLN117 — the host half of loft's browser thread pool: what turns `par` into
// real Web Worker threads.  One source, two embeddings — a module for the
// gallery bundle, and inlined into the single-file `loft --html` page (which
// strips the trailing `export`, the same way loft-asyncify.js is inlined).
//
// Two kinds of loft wasm use this, and the difference is one line in the worker:
//
//   raw (`loft --html`)   — instantiate the module directly, then move this
//                           thread onto its own stack + TLS block.
//   wasm-bindgen gallery  — `import(mainJS)` and let the generated glue do it.
//
// Everything else — who spawns, in what order, and the pool itself — is shared.
//
// Usage:
//   const memory = loftSharedMemory(wasmBytes);     // BEFORE instantiating
//   const { instance } = await WebAssembly.instantiate(module, { env: { memory }, ... });
//   await startLoftWorkers(instance.exports, navigator.hardwareConcurrency, { module, memory });
//   instance.exports.loft_start();
//
// …or, for the gallery, `startLoftWorkers(wasm, n, { memory, mainJS })`.
//
// Without any of this the wasm still runs: rayon falls back to a single-threaded
// pool and `par` runs sequentially, with identical results (@PLN117 arc D).  A
// host that is not cross-origin isolated has no SharedArrayBuffer, so that is
// exactly what it gets.

// Each worker's shadow stack.  wasm-ld gives the main thread 1 MiB; matching it
// keeps a deep loft recursion behaving the same wherever it runs.
const LOFT_WORKER_STACK = 1 << 20;

// The worker body, inlined so a self-contained page stays one file.  The stack
// and TLS setup runs before anything else on this thread: a fresh instance
// starts with the SAME stack pointer as the main thread, so until it is moved
// off it, running any wasm here corrupts the page.
const LOFT_WORKER_SOURCE = `
self.onmessage = async (e) => {
  const { module, memory, handoff, stackTop, tlsBase, mainJS } = e.data;
  let exports;
  try {
    if (mainJS) {
      // wasm-bindgen bundle: its glue owns instantiation, including this
      // thread's stack and TLS.
      const pkg = await import(mainJS);
      exports = await pkg.default({ module_or_path: new URL('loft_bg.wasm', mainJS), memory });
    } else {
      // Raw bundle.  Every import has to be present or the instance will not
      // link, and a worker has none of the page's host objects (no canvas, no
      // DOM).  So each one gets a stub: a worker runs loft bytecode, and a host
      // call from inside a par body no-ops with a warning rather than taking the
      // whole page down.
      const imports = { env: { memory } };
      let warned = false;
      for (const im of WebAssembly.Module.imports(module)) {
        if (im.kind !== 'function') continue;
        (imports[im.module] = imports[im.module] || {})[im.name] = () => {
          if (!warned) {
            warned = true;
            console.warn('loft: host call ' + im.module + '.' + im.name + ' is not available on a worker thread');
          }
        };
      }
      const result = await WebAssembly.instantiate(module, imports);
      exports = (result.instance || result).exports;
      exports.__stack_pointer.value = stackTop;
      if (tlsBase) exports.__wasm_init_tls(tlsBase);
    }
  } catch (err) {
    self.postMessage({ loftWorker: 'failed', error: String((err && err.message) || err) });
    return;
  }
  self.postMessage({ loftWorker: 'ready' });
  try {
    exports.loft_rayon_start_worker(handoff);   // runs until the page ends
  } catch (err) {
    self.postMessage({ loftWorker: 'failed', error: String((err && err.message) || err) });
  }
};`;

/**
 * Build the shared memory a threaded loft wasm imports as `env.memory`.
 *
 * The initial size has to be at least what the module declares, so it is read
 * off the module's own import section rather than guessed.  Returns null when
 * the wasm is not a threaded build (it declares no memory import) or the host
 * cannot share memory, which is the caller's signal to run single-threaded.
 */
function loftSharedMemory(wasmBytes) {
  const limits = loftMemoryImportLimits(wasmBytes);
  if (!limits) return null;
  try {
    return new WebAssembly.Memory({
      initial: limits.initial,
      maximum: limits.maximum || 16384,
      shared: true
    });
  } catch (e) {
    return null;
  }
}

/**
 * The `(initial, maximum)` page counts of the wasm's imported memory, or null if
 * it imports none.  A minimal walk of the import section — enough to size the
 * memory, not a wasm parser.
 */
function loftMemoryImportLimits(wasmBytes) {
  const bytes = wasmBytes instanceof Uint8Array ? wasmBytes : new Uint8Array(wasmBytes);
  let p = 8;                                        // skip magic + version
  const u32 = () => {                               // LEB128
    let result = 0, shift = 0, b;
    do { b = bytes[p++]; result |= (b & 0x7f) << shift; shift += 7; } while (b & 0x80);
    return result >>> 0;
  };
  while (p < bytes.length) {
    const id = bytes[p++], size = u32(), end = p + size;
    if (id === 2) {                                 // import section
      const count = u32();
      for (let i = 0; i < count; i++) {
        // Read the length first: `p += u32()` would add it to the OLD p, because
        // JS reads the left side before the right side advances it.
        const moduleLen = u32(); p += moduleLen;
        const fieldLen = u32(); p += fieldLen;
        const kind = bytes[p++];
        if (kind === 0) { u32(); }                  // function: type index
        else if (kind === 1) { p++; const f = bytes[p++]; u32(); if (f) u32(); }   // table
        else if (kind === 2) {                      // memory — what we came for
          const flags = bytes[p++];
          const initial = u32();
          const maximum = (flags & 0x01) ? u32() : 0;
          return { initial, maximum };
        } else if (kind === 3) { p++; p++; }        // global
      }
    }
    p = end;
  }
  return null;
}

// Spawned workers are held here so the browser does not collect them out from
// under the pool they are running.
const loftWorkers = [];

/**
 * Start the pool: claim the handoff, spawn `threads` workers, wait for them to
 * come up, then install rayon's global pool over them.  Resolves to the number
 * of worker threads actually running — 0 when the page runs single-threaded,
 * which is a supported outcome and not an error.
 *
 * The three steps have to be in that order and cannot be collapsed: installing
 * the pool waits for the workers to prime, and the workers cannot finish booting
 * until the main thread returns to the event loop.
 *
 * `where` is `{ memory }` plus either `module` (a raw bundle) or `mainJS` (the
 * URL of a wasm-bindgen bundle's glue).
 *
 * Every failure path resolves to 0 rather than throwing: a page whose host
 * cannot thread must still run its program.
 */
async function startLoftWorkers(exports, threads, where) {
  if (!self.crossOriginIsolated || !exports.loft_pool_new || !(threads > 1)) return 0;
  try {
    const handoff = exports.loft_pool_new(threads);
    if (!handoff) return 0;
    const { module = null, memory, mainJS = null } = where;
    const url = URL.createObjectURL(new Blob([LOFT_WORKER_SOURCE], { type: 'text/javascript' }));
    const tlsSize = !mainJS && exports.__tls_size ? exports.__tls_size.value : 0;
    const tlsAlign = !mainJS && exports.__tls_align ? exports.__tls_align.value : 8;
    const pending = [];
    for (let i = 0; i < threads; i++) {
      // Allocated here, on the one thread that is currently running wasm: a
      // worker cannot allocate its own stack without first standing on one.
      const stack = mainJS ? 0 : exports.loft_thread_alloc(LOFT_WORKER_STACK, 16);
      const tls = tlsSize ? exports.loft_thread_alloc(tlsSize, tlsAlign) : 0;
      if (!mainJS && (!stack || (tlsSize && !tls))) {
        throw new Error('loft: out of memory for a worker stack');
      }
      const worker = new Worker(url, mainJS ? { type: 'module' } : undefined);
      pending.push(new Promise((resolve, reject) => {
        worker.onmessage = (e) => {
          if (e.data && e.data.loftWorker === 'ready') resolve();
          else if (e.data && e.data.loftWorker === 'failed') reject(new Error(e.data.error));
        };
        worker.onerror = (e) => reject(new Error(e.message || 'loft worker failed'));
      }));
      worker.postMessage({
        module, memory, handoff, mainJS,
        stackTop: stack + LOFT_WORKER_STACK, tlsBase: tls
      });
      loftWorkers.push(worker);
    }
    await Promise.all(pending);
    URL.revokeObjectURL(url);
    return exports.loft_pool_build() === 0 ? threads : 0;
  } catch (e) {
    console.warn('loft: threading unavailable, running par sequentially —', e.message || e);
    return 0;
  }
}

/**
 * A `TextDecoder` that also reads SHARED memory.
 *
 * `TextDecoder.decode` refuses a view onto a SharedArrayBuffer, and a threaded
 * build's whole linear memory is one — so every host call that reads a loft
 * string (print, output, deliver, …) would throw on a threaded page and nowhere
 * else.  Copying the bytes out first costs one small allocation per host call
 * and makes the two kinds of page behave identically.
 */
function loftTextDecoder() {
  const decoder = new TextDecoder();
  return {
    decode(view) {
      return decoder.decode(view.buffer instanceof ArrayBuffer ? view : view.slice());
    }
  };
}

/**
 * Instantiate a loft browser wasm — threaded when the build and the host both
 * allow it, plain when either doesn't.  This is the one place a loft page starts
 * from, so "how does this page get its memory and its threads" has a single
 * answer no matter which shell embeds it.
 *
 * Resolves to `{ instance, memory, threads }`; `memory` is null for a
 * single-threaded build (its memory is the instance's own export) and `threads`
 * is 0 when `par` will run sequentially.
 */
async function loftInstantiate(wasmBytes, imports, maxThreads) {
  const module = await WebAssembly.compile(wasmBytes);
  // A threaded build imports its memory, so the memory must exist — and be
  // shared — before the instance does.
  const memory = loftSharedMemory(wasmBytes);
  if (memory) imports.env = Object.assign(imports.env || {}, { memory });
  const result = await WebAssembly.instantiate(module, imports);
  const instance = result.instance || result;
  const threads = memory
    ? await startLoftWorkers(instance.exports, maxThreads || navigator.hardwareConcurrency,
                             { module, memory })
    : 0;
  // How many worker threads this page got — 0 means `par` runs sequentially.
  // A page that expected parallelism cannot otherwise tell: results are the same
  // either way, only slower.  `?loftTrace=1` goes further and has loft report
  // how many workers each individual `par` really dispatched across.
  globalThis.loftThreads = threads;
  if (instance.exports.loft_set_par_trace &&
      typeof location !== 'undefined' && /[?&]loftTrace=1/.test(location.search)) {
    instance.exports.loft_set_par_trace(1);
  }
  return { instance, memory, threads };
}

// One line, and the last one: `loft --html` inlines this file into a non-module
// <script> by deleting exactly this statement (as it does for the other glue).
export { startLoftWorkers, loftInstantiate, loftTextDecoder, loftSharedMemory, loftMemoryImportLimits };
