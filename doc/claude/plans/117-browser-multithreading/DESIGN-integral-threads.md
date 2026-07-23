<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# @PLN117 — Integral wasm threading (design, pre-implementation)

Companion to [`README.md`](README.md) / [`DESIGN.md`](DESIGN.md). Written **before code**
(design-protocol): the failure paths and the re-assertion count are where the invariant
becomes nameable. Status: **Phase 0 (POC) DONE — keystone claim CONFIRMED, design amended;
implementation in progress.**

## Phase 0 result — the keystone holds, three amendments

The throwaway POC (a raw `cdylib`: rayon + loft's `spawn_handler` over a raw
`loft_thread.spawn_workers` import, built with `build-std` + shared memory, driven across Blob
Web Workers by a **non-module** `<script>`) **works**: imports are exactly
`{env.memory, loft_thread}` — no `__wbindgen_placeholder__` — and it reports
`distinct_workers=4`, value identical to the sequential run (`sum=36023`, the same number loft's
own `par-thread-proof` gate produces from the equivalent loft program). The sequential fallback
(no pool) gives `distinct_workers=1` and the same value. **loft's `par` runtime needs no
wasm-bindgen.**

It also falsified three things the pre-implementation design got wrong. Each was invisible on
paper and cost minutes to find in the POC:

- **A1 — `rayon/web_spin_lock` is mandatory, and loft must supply its own `wasm_sync`.**
  A page's main thread may not block: any `memory.atomic.wait32` there throws *"Atomics.wait
  cannot be called in this context"*. rayon reaches its internal `Mutex`/`Condvar` from the
  calling thread in two places (`build_global` → `wait_until_primed`, and `in_worker_cold`'s
  `LockLatch` — the join every `par` from the main thread performs), so a stock-`std` rayon dies
  on the first `par`. rayon's `web_spin_lock` feature fixes this by swapping `std::sync` for the
  `wasm_sync` crate — which is exactly what makes the *existing* gallery work (wasm-bindgen-rayon
  depends on `rayon` with `features = ["web_spin_lock"]`; nothing in loft said so). But
  crates.io's `wasm_sync` detects the main thread with `web_sys::window()` — **wasm-bindgen
  again**, which the raw path forbids. So loft ships its own drop-in `wasm_sync` (in-tree
  `wasm-sync/`, wired with `[patch.crates-io]`): same spin-instead-of-block behaviour, no
  wasm-bindgen. Its `can_block` defaults to *true* (plain `std`, today's semantics) and loft
  marks the **main** thread non-blocking — fail-open, so a missed mark throws loudly rather than
  silently burning CPU.
- **A2 — pool init must be two-phase (JS-driven), not one wasm call.** The design's C2 shape
  (`init_thread_pool` calls the spawn import, then `build_global`) **deadlocks**: `build_global`
  waits for the workers to prime while the main thread is still inside the call, so the workers
  never finish booting. Proven both ways in the POC — one-shot hangs, two-phase returns in 3 ms.
  The runtime therefore exposes `loft_pool_new(n)` (create the handoff + ask the host to spawn)
  and `loft_pool_build()` (install the global pool); the JS bootstrap awaits every worker's
  *ready* message in between. This is the ordering `wasm-bindgen-rayon`'s `startWorkers` uses,
  and it is the reason it uses one.
- **A3 — each worker needs its own shadow stack + TLS block, set from JS.** Every
  `WebAssembly.Instance` gets its own copy of the mutable globals but they all start at the *same*
  value, so without this every worker tramples the main thread's shadow stack. wasm-bindgen's
  thread transform did this silently; the raw path must do it itself: export `__stack_pointer`
  (one more link-arg than `make wasm-mt` passes), have the wasm export an allocator
  (`loft_thread_alloc`), and let the worker bootstrap set `__stack_pointer` and call
  `__wasm_init_tls` on per-worker blocks before running the worker loop. Also: the shared memory
  is imported as **`env.memory`**, so `--html`'s import allow-list must admit `env` and
  `loft_thread`.

**D1 is decided by probe: (b), `rustc`-direct + `--sysroot`.** An atomics std sysroot assembled
from the `build-std` output (copy the `deps/*.rlib` into
`<sysroot>/lib/rustlib/wasm32-unknown-unknown/lib/`) compiles a shared-memory cdylib with
`rustc` directly — verified. That keeps the `--html` pipeline, its single loft copy, and the
wasm-bridge crates exactly as they are; option (a) (generated cargo crate) would have had to
re-solve the crate-dup guard for no gain.

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

## Phased plan — small, safe, gated steps (each independently landable)

Each step names its **exact code points** and the **gate** that proves it before the next starts.
The ordering keeps the working gallery (wasm-bindgen-rayon) untouched until its loft-runtime
replacement is green (F7). Steps 0–2 are purely additive (feature-gated, default OFF → zero risk to
existing builds); the first behaviour change to a shipped path is step 4.

### Phase 0 — POC the keystone (throwaway; nothing committed to the repo)

- **0.1 — a raw threaded wasm, no wasm-bindgen.** In a scratch dir (`$CLAUDE_JOB_DIR/tmp/poc/`), a
  standalone `cdylib`: rayon + a `ThreadPoolBuilder::spawn_handler(|t| { sender.send(t); Ok(()) })
  .build_global()` (copied from `wasm-bindgen-rayon-1.3.0/src/lib.rs::build`), a raw import
  `#[link(wasm_import_module = "loft_thread")] unsafe extern "C" { fn loft_thread_spawn_workers(n:
  u32, receiver: *const u8); }` (model: `src/wasm_assets.rs:28`), and `#[unsafe(no_mangle)] pub
  extern "C" fn loft_rayon_start_worker(receiver: *const u8)` that pulls a `ThreadBuilder` and runs
  it. Build with the proven `make wasm-mt` recipe (nightly + `-Z build-std=panic_abort,std` + the
  `WASM_MT_RUSTFLAGS` link-arg set in the `Makefile`).
  - **Gate:** builds; `native_utils::html_wasm_import_modules` reports the imports are exactly
    `{loft_thread}` — **no** `__wbindgen_placeholder__`. Proves loft threading needs no wasm-bindgen.
- **0.2 — drive it across Blob workers.** A throwaway JS harness (in `$…/tmp/poc/`): create a shared
  `WebAssembly.Memory({shared:true,…})`, instantiate; implement `loft_thread.spawn_workers(n,recv)`
  by spawning `n` Blob-URL workers, each `postMessage`'d `{module, memory, recv}` -> the worker
  instantiates the module against the shared memory and calls `loft_rayon_start_worker(recv)`; then
  run a `.into_par_iter().sum()` and read the distinct-worker count (as `par-thread-proof.html`
  does). Serve COOP/COEP via `tests/wasm/coi-server.py`, drive headless.
  - **Gate:** `distinct_workers >= 2` + correct sum on a RAW bundle. **If this fails, the design
    changes here — before any pipeline surgery.**

### Phase 1 — the build recipe (C1), additive, feature-gated

- **1.1 — a `wasm-native-threads` Cargo feature.** `Cargo.toml` `[features]`: add
  `wasm-native-threads = ["threading"]` (NOT `wasm` — this is the raw, wasm-bindgen-free path).
  `default` unchanged.
  - **Gate:** `cargo build` (default) unchanged; `cargo check --features wasm-native-threads`
    compiles (nothing consumes it yet).
- **1.2 — a threaded runtime-rlib shape.** `src/native_utils.rs`: add
  `WasmRuntimeShape::HtmlThreads` beside `Html` (~line 138) — same triple
  (`wasm32-unknown-unknown`), `features()` = `"random wasm-native-threads"` (~line 179),
  `isolated_target_subdir()` = `Some("target/loft/html-mt")` (~line 189). In
  `ensure_loft_runtime_rlib` (~line 263) the cargo command for this shape gains
  `rustup run nightly` + `-Z build-std=panic_abort,std` + `RUSTFLAGS=<WASM_MT_RUSTFLAGS>`.
  - **Gate:** `ensure_loft_runtime_rlib(HtmlThreads)` produces a threaded `libloft.rlib` + an
    atomics-std under `target/loft/html-mt/`; the existing `Html` shape build command is untouched.

### Phase 2 — loft's threading runtime (C2), behind the feature

- **2.1 — `src/wasm_threads.rs` (new), `#[cfg(feature = "wasm-native-threads")]`.** loft's
  `PoolBuilder` (a `std`/`crossbeam` channel + the spawn_handler from 0.1), `pub fn
  init_thread_pool(n: usize)` that spawns via the `loft_thread_spawn_workers` import then
  `build_global()`, the raw import block, and the `loft_rayon_start_worker` export. Register the
  module in `src/lib.rs`. Mirrors `wasm-bindgen-rayon-1.3.0/src/lib.rs` with the wasm-bindgen
  boundary swapped for the raw import.
  - **Gate:** `cargo check --features wasm-native-threads` clean; default build unchanged.
- **2.2 — confirm `with_pool` already covers this path.** `src/parallel.rs::with_pool` (~line 154)
  runs `f()` (the global pool) under `#[cfg(feature = "wasm")]`; broaden that arm's cfg to
  `any(feature = "wasm", feature = "wasm-native-threads")` so the raw path also uses the global
  pool loft's `init_thread_pool` installs. `rayon_pool()` stays `not(wasm)`-gated — extend to
  `not(any(wasm, wasm-native-threads))`.
  - **Gate:** `loft introspect` on a `par` fixture is **byte-identical** before/after on both
    backends (with_pool is runtime, not emitted); `cargo test --test threading` 47/47 (default,
    feature off); `cargo check --features wasm-native-threads` clean.

### Phase 3 — the JS worker bootstrap (C3), one source

- **3.1 — `doc/loft-thread.js` (new).** `installLoftThreads(imports, getModule, getMemory)` adds a
  `loft_thread` key to the wasm imports object implementing `spawn_workers(n, recv)`; plus the
  inlined worker body (instantiate `module` vs shared `memory`, call `loft_rayon_start_worker`).
  Written to be usable **both** as a file (gallery) and `include_str!`-inlined (`--html`), matching
  how `doc/loft-gl-wasm.js` is consumed today (`src/main.rs:6989`).
  - **Gate:** `node --check doc/loft-thread.js`; loads without error.
- **3.2 — prove the four gates on a loft-runtime RAW bundle.** Build a raw threaded bundle (the
  Phase-1 recipe) wired to `doc/loft-thread.js`, and run the existing gates against it.
  - **Gate:** `par-thread-proof.sh`, `par-memory-proof.sh`, `par-scaling-bench.sh`,
    `par-ui-responsive.sh` all PASS on the loft-runtime bundle (parity with the wasm-bindgen-rayon
    numbers). This retires the keystone risk for real bundles.

### Phase 4 — `--html` threaded (the payoff; first change to a shipped path)

- **4.1 — atomics-std for `--html` (D1=a).** Switch the `prog.wasm` build (`src/main.rs` ~6609,
  the `rustc` invocation) to a generated **cargo** crate built with `-Z build-std` + the atomics
  link-args, `--extern loft=<HtmlThreads rlib>`. Re-solve the crate-dup guard (the reason for
  `rustc`-direct, `src/main.rs` ~6643): pin loft as a single path/rlib dep so exactly one copy
  links. Extend the import allow-list `native_utils::html_wasm_import_modules_ok` (~line 587) to
  permit `loft_thread`.
  - **Gate:** `loft --html` on a par-using program emits a wasm importing ONLY
    `{loft_gl, loft_io, loft_thread}`; it instantiates in a COOP/COEP dev server.
- **4.2 — inline the bootstrap + init.** Add `include_str!("../doc/loft-thread.js")` beside the
  gl glue (`src/main.rs` ~6989); in the emitted non-module driver (`src/main.rs` ~7037) call the
  COI-gated `init_thread_pool(navigator.hardwareConcurrency)` before `loft_start`. asyncify note:
  the worker loop must not be on an asyncify-suspended path (workers run `loft_rayon_start_worker`,
  not `loft_start`) — verify the `--asyncify` pass (~6855) leaves the worker export intact.
  - **Gate:** a par-using `--html` page threads on a COOP/COEP host (`distinct_workers >= 2`) and
    runs sequential on a plain host (never breaks — the arc-D fallback, in a self-contained file).
- **4.3 — the opt-out (D2).** `src/generation/mod.rs`: set an `Output.uses_par` flag when emitting
  any `n_parallel_*` call (the list at ~line 116). `--html` takes the threaded path only when
  `uses_par` (else today's single-threaded `rustc`-direct path, unchanged); add `--threads` /
  `--no-threads` overrides in the CLI arg parse (`src/main.rs` ~4737).
  - **Gate:** a par-free `--html` page is byte-identical to today's bundle; a par-using one is
    threaded; both render.

### Phase 5 — migrate the gallery, drop wasm-bindgen-rayon

- **5.1 — gallery on loft's runtime.** Point `make gallery-mt` / `make wasm-mt` at
  `wasm-native-threads` + `doc/loft-thread.js`; the gallery loaders (`doc/playground.html`,
  `doc/gallery-run.html`) call loft's `init_thread_pool` instead of wasm-bindgen-rayon's.
  - **Gate:** the four headless gates still green on the gallery bundle.
- **5.2 — remove the dependency.** Drop `wasm-bindgen-rayon` from `Cargo.toml` (~line 95, incl. the
  `no-bundler` feature), the re-export in `src/wasm.rs`, and any wasm-bindgen-rayon-only Makefile
  flags.
  - **Gate:** `cargo build` clean; `Cargo.lock` no longer lists `wasm-bindgen-rayon`; the four gates
    green. loft now owns its `par` runtime end-to-end.

## Open decisions for sign-off

- **D1** `--html` std: (a) generated-cargo-crate + build-std, or (b) `rustc`-direct + `--sysroot`
  atomics-std. (Recommend **a** — dissolves the std problem; costs re-solving the crate-dup guard.)
- **D2** opt-out default: auto-detect `par` usage → threads on (recommend), vs default-on +
  `--no-threads`.
- **D3** scope of "every wasm build": browser paths (gallery + `--html`) now; `--native-wasm`
  (wasip2, wasi-threads — a *different* thread model) explicitly out of scope for this arc.
