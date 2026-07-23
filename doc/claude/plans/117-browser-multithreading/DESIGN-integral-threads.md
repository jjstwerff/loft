<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# @PLN117 — Integral wasm threading (design, pre-implementation)

Companion to [`README.md`](README.md) / [`DESIGN.md`](DESIGN.md). Written **before code**
(design-protocol): the failure paths and the re-assertion count are where the invariant
becomes nameable. Status: **DESIGN — awaiting sign-off. Not implemented.**

## The ask

`par` / `par_fold` are **base language features**, so browser threading must be an **integral
part of how loft builds wasm**, not bolted onto `loft --html`. The gallery got threading the
easy way (wasm-bindgen-rayon); that does **not** generalise, and the goal is one mechanism that
every wasm build path shares, with an **opt-out** for a program that provably uses no `par`.

## The invariant (the whole design in one sentence)

> **Every loft wasm build produces a binary whose `par` runs on Web Worker threads over shared
> memory, because threading is ONE runtime component (a rayon pool whose workers are spawned
> through loft's own host-bridge ABI, `loft_thread.*`, independent of wasm-bindgen) plus ONE
> build recipe (atomics-compiled std via `build-std` + shared/imported memory + the TLS/heap
> exports). A build turns threading on by setting a single flag; a `par`-free program turns it
> off for a smaller single-threaded bundle. `par` behaves identically everywhere because all
> paths share the same `with_pool` dispatch and the same worker runtime.**

If that rule holds, a build path we never touched (a future `--native-wasm` browser mode, a new
export) threads correctly for the same reason the tested ones do: it sets the flag and links the
one runtime.

## Why the naive "add it to `--html`" is architecturally blocked (all confirmed)

1. **`--html` is wasm-bindgen-FREE by design.** The Html-shape runtime rlib is built
   `--no-default-features --features random` (`src/native_utils.rs`), and `--html` **hard-rejects**
   any wasm that imports `__wbindgen_placeholder__` (`src/main.rs` ~6914) — its embedded glue only
   provides raw `loft_gl` / `loft_io` externs, and it uses **asyncify** for frame-yield. So
   `wasm-bindgen-rayon` (a wasm-bindgen crate) cannot be used in `--html` at all.
2. **`--html` builds via `rustc` directly, so it cannot `-Z build-std`.** `build-std` is a *cargo*
   flag. Probe (confirmed): `rustc --target wasm32-unknown-unknown -Ctarget-feature=+atomics
   -Clink-arg=--shared-memory …` on a trivial crate →
   `rust-lld: error: --shared-memory is disallowed by std…because it was not compiled with
   'atomics' or 'bulk-memory' features`. An **atomics-compiled std is mandatory** for *any* threaded
   wasm, and only cargo (`build-std`) or a hand-built sysroot can supply it.
3. **`--html` is a self-contained single file** (wasm inlined as base64 in a non-module `<script>`,
   `src/main.rs` ~6929). wasm-bindgen-rayon's worker bootstrap wants ESM + a separate
   `workerHelpers.js` fetched via `import.meta.url` — incompatible with one inlined file.

These are why the mechanism must be **loft's own**, not the third-party crate.

## The architecture — three components, each with ONE home

### C1 — the build recipe (shared by every path)

A single knob `wasm_threads: bool` on the wasm-build entry point turns on, together:
- **cargo `-Z build-std=panic_abort,std`** on nightly with `rust-src` → an atomics-compiled std.
  The runtime-rlib build (`native_utils::ensure_loft_runtime_rlib`) is **already cargo** — it gains
  the `build-std` + atomics flags, so the atomics-std lands in the same isolated target-dir the
  build already produces and caches (one-time cost, fingerprinted).
- the link-arg set proven in `make wasm-mt`: `+atomics,+bulk-memory,+mutable-globals`,
  `--shared-memory --max-memory=… --import-memory --export=__heap_base --export=__wasm_init_tls
  --export=__tls_{size,align,base}`.
- **`--html` gets its std via this same build-std output**: either (a) switch `prog.wasm` from
  `rustc`-direct to a generated **cargo** crate built with `-Z build-std` (cleanest; dissolves the
  std problem, but must re-solve the loft-crate-duplicate reason `rustc`-direct exists — a pinned
  path-dep + one build unit), or (b) keep `rustc`-direct and pass `--sysroot <atomics-std sysroot
  assembled from the build-std output>`. **Decision pending — (a) is cleaner, (b) is less
  disruptive.** This is the one genuinely-hard sub-choice; it is isolated to the `--html` build
  command.

### C2 — loft's threading runtime (replaces wasm-bindgen-rayon)

rayon stays; only the **spawn glue** becomes loft's own. wasm-bindgen-rayon is ~130 lines whose
core is `ThreadPoolBuilder::new().spawn_handler(|thread| { sender.send(thread); Ok(()) })
.build_global()` — the JS spawns N workers, each calls `wbg_rayon_start_worker(receiver)` which
receives a `ThreadBuilder` and runs it. loft replicates this with **raw host imports** instead of
`#[wasm_bindgen]`:
- `loft_thread.spawn_workers(n, receiver_ptr)` — a raw import (same family as `loft_gl.*` /
  `loft_io.*`); JS spawns `n` Web Workers, handing each the compiled module + the shared memory
  (JS already holds both) + `receiver_ptr`.
- `loft_rayon_start_worker(receiver_ptr)` — a raw export each worker calls; receives a
  `ThreadBuilder` from the channel and runs it.
- `init_thread_pool(n)` — loft's own, no wasm-bindgen. Idle/uninitialised ⇒ 1 thread ⇒ the proven
  sequential fallback (arc D).

This lives in loft's wasm runtime (`src/wasm.rs` / a new `src/wasm_threads.rs`), so it is present in
**both** the wasm-bindgen gallery build and the raw `--html` build — one component, no
wasm-bindgen coupling (own-your-dependencies; the `par` runtime belongs to loft, not to a crate
that only works under one ABI).

### C3 — the JS worker bootstrap (one module, two embeddings)

The worker spawns a Blob URL of an inlined worker script; main `postMessage`s `{module, memory,
receiver_ptr}` (a `WebAssembly.Module` and a shared `Memory` are both structured-cloneable); the
worker instantiates the module against the shared memory and calls `loft_rayon_start_worker`. The
SAME bootstrap source is (a) a file in the gallery JS runtime (`doc/loft-rt.js`) and (b) inlined
into the `--html` single file — so self-contained stays self-contained.

## Re-assertion sites — the prospective tell

"This wasm is threaded" must be re-stated at: (1) the std, (2) the memory/link-flags, (3) the
rayon dispatch, (4) the worker spawn, (5) the JS bootstrap. The design **collapses** them:
(1)+(2) become the single `wasm_threads` flag on the build recipe (C1); (3) is already ONE place
(`parallel.rs::with_pool`); (4)+(5) are the one runtime component (C2+C3). So each *path* asserts
the invariant **once** (set the flag + link the runtime) and omission is **loud** (miss the
atomics-std → the linker errors, as the probe showed; it is not a silent wrong answer). `N × silence`
≈ 0.

## Failure paths (enumerated → each has an owner)

- **F1 std forgotten** → `--shared-memory disallowed` link error. *Loud (compile-time). Good.*
- **F2 pool never initialised** (JS didn't spawn) → 1-thread pool → sequential. *Never crashes
  (arc D, proven).*
- **F3 `par`-free program ships the heavier COOP/COEP-requiring bundle** → the opt-out: default the
  flag OFF and auto-enable when the compiled program contains any `par`/`par_fold` (loft already
  knows this at codegen — the boolean the ask mentions), OR default ON with an explicit
  `--no-threads`. *Decision pending; auto-detect is the clean default.*
- **F4 self-contained worker needs module+memory** → passed by `postMessage` (both
  structured-cloneable). *Proven pattern (wasm-bindgen-rayon does exactly this).*
- **F5 non-COI host** → shared memory still instantiates (proven: `new WebAssembly.Memory({shared})`
  works even when `SharedArrayBuffer` is absent), workers can't spawn → sequential. *Never breaks
  (proven this session).*
- **F6 build-std cost** → cached + fingerprinted by the existing runtime-rlib mechanism. *One-time.*
- **F7 dropping wasm-bindgen-rayon regresses the PROVEN gallery** → **phase it**: build + prove
  loft's runtime on the raw/`--html` path FIRST; migrate the gallery to loft's runtime only once
  the four headless gates (`par-*-proof.sh`, `par-scaling-bench.sh`, `par-ui-responsive.sh`) pass on
  it; drop wasm-bindgen-rayon last. The working gallery is never disturbed before its replacement is
  green.

## The one load-bearing claim to POC before committing

**Claim:** a raw wasm cdylib (no wasm-bindgen) with rayon + a loft `spawn_handler` over a raw
`loft_thread.spawn_workers` import, built with build-std + shared memory, actually parallelises a
`.into_par_iter()` across Blob Web Workers in a browser.

It is *low-risk* (it is wasm-bindgen-rayon's proven mechanism with the wasm-bindgen boundary swapped
for raw imports), but it is the design's keystone, so **implementation step 0 is a throwaway POC of
exactly this** — if it fails, the design changes here, cheaply, before any pipeline surgery.

## Phased plan (each phase independently landable + gated)

0. **POC** the C2/C3 keystone (raw rayon-in-wasm across Blob workers). Gate: ≥2 distinct workers,
   correct value — the `par-thread-proof` shape, on a raw bundle.
1. **C1 build recipe**: `wasm_threads` flag + build-std/atomics wired into the runtime-rlib build;
   cache. Gate: a threaded raw rlib builds; the linker accepts shared memory.
2. **C2 loft threading runtime** (`init_thread_pool`, `spawn_workers`, `start_worker`, the
   spawn_handler) behind the flag. Gate: native suite green; `par` unchanged when the flag is off.
3. **C3 JS bootstrap** (gallery file + `--html` inline). Gate: the four headless gates pass on the
   loft-runtime bundle.
4. **`--html` threaded** (C1 decision a/b + inline C3 + the auto-detect opt-out). Gate: a
   `par`-using `--html` page threads on a COOP/COEP host and runs sequential elsewhere; a `par`-free
   page is byte-for-byte the current single-threaded bundle.
5. **Migrate the gallery** to loft's runtime; **drop wasm-bindgen-rayon**. Gate: all gates still
   green; `Cargo.toml` no longer depends on `wasm-bindgen-rayon`.

## Open decisions for sign-off

- **D1** `--html` std: (a) generated-cargo-crate + build-std, or (b) `rustc`-direct + `--sysroot`
  atomics-std. (Recommend **a** — dissolves the std problem; costs re-solving the crate-dup guard.)
- **D2** opt-out default: auto-detect `par` usage → threads on (recommend), vs default-on +
  `--no-threads`.
- **D3** scope of "every wasm build": browser paths (gallery + `--html`) now; `--native-wasm`
  (wasip2, wasi-threads — a *different* thread model) explicitly out of scope for this arc.
