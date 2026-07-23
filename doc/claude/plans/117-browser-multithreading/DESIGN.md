<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# @PLN117 — detailed design: small, safe, gated steps

Companion to [`README.md`](README.md). This is the step plan — each step is small,
independently landable, names its exact code points, and states the gate that proves it
before the next step starts. Nothing here is speculative wiring: it is grounded in the code
as it stands today.

## The decision that unblocks everything (open-question 1, resolved)

There are **two half-built wasm-threading designs in the tree**, and step 0 is to pick one:

1. **rayon + `wasm-bindgen-rayon`** — `Cargo.toml` declares `wasm-bindgen-rayon` under the
   `wasm-threads` feature, but **nothing in `src/` ever calls its `init_thread_pool`** (grep is
   empty), and `parallel.rs::rayon_pool()` builds a *private* `rayon::ThreadPoolBuilder::new()
   .build()` — which under wasm has no way to spawn (wasm-bindgen-rayon works by installing a
   **global** pool with a Web-Worker `spawn_handler`; a private builder can't spawn OS threads
   that don't exist). So the rayon path is declared-but-unwired.
2. **hand-rolled workers** — `src/wasm.rs::worker_entry` (a **no-op stub**, W1.18-2),
   `tests/wasm/worker.mjs` + `tests/wasm/parallel.mjs::LoftThreadPool` (W1.18-3): workers park
   on a `SharedArrayBuffer` control signal and call `worker_entry`, which does nothing.

**Decision: commit to rayon + `wasm-bindgen-rayon`; retire the hand-rolled model.** Rationale:
the native path is *already* rayon (`parallel.rs`), so this gives ONE scheduler story (the
`parallel.rs:140` comment's stated intent), the `.into_par_iter()` dispatch code is reused
untouched, and wasm-bindgen-rayon is the maintained, tested rayon-on-wasm bridge. The
hand-rolled worker glue is deleted, not finished. Everything below follows from this.

**The one invariant to protect** (an **S**-category hazard — silent corruption): *under wasm
shared linear memory, a `par` produces byte-identical results to the sequential fallback, with
no worker ever writing a store another worker (or the parent) reads.* On native this is carried
by @PLN108 (read-only-share + the dispatcher joining before the parent drops, ASan/TSan-clean).
Shared linear memory changes the argument — so it must be **re-proven with a falsification
instrument**, not assumed.

## Track 1 — a threaded NODEJS build that provably parallelises

Nodejs first: it needs no COOP/COEP (Node gives you `SharedArrayBuffer` + worker_threads
directly), so it is the fastest correctness loop. Every step lands + verifies on `pkg-mt`.

### Step 0 — the falsification instrument (before any code change)

Build the baseline that a working threaded build must change, and that a *broken* one fails.

- Write `tests/wasm/scripts/par-parallelism.loft`: a `par(b = work(a), N)` over K elements
  where `work` is CPU-heavy AND each worker records its OS/worker identity into an output slot
  (so we can *prove* ≥2 distinct workers ran, not just infer from timing). Assert the numeric
  result is correct (value gate) AND that `distinct_worker_ids ≥ 2` (parallelism gate).
- **Code points:** new `.loft` under `tests/wasm/scripts/`; run via `make wasm-mt` then
  `node tests/wasm/suite.mjs --threaded par-parallelism.loft`.
- **Gate:** on TODAY's tree this test must show `distinct_worker_ids == 1` (sequential) —
  confirming the instrument can see the difference. If it already shows ≥2, the tree is more
  wired than believed and the plan re-scopes. A no-output / crash cell is vacuous — reject it.

### Step 1 — export `initThreadPool` from the wasm module

- Add, under `#[cfg(feature = "wasm-threads")]`, the wasm-bindgen-rayon pool export:
  `pub use wasm_bindgen_rayon::init_thread_pool;` (its macro generates the `initThreadPool`
  JS export + the worker startup shim). **Code point:** `src/wasm.rs` (near the other
  `#[wasm_bindgen]` exports); the dep is already in `Cargo.toml:95`.
- **Gate:** `make wasm-mt` succeeds and `pkg-mt/loft.js` exports `initThreadPool`. No behaviour
  change yet (nothing calls it).

### Step 2 — make `rayon_pool()` use the wasm global pool

- `parallel.rs::rayon_pool()` (line 195) currently returns a private `&'static ThreadPool`.
  Under `#[cfg(feature = "wasm")]` it must instead run on the **global** pool that
  `initThreadPool` built. Cleanest: keep the `rayon_pool()` signature for native, but gate the
  call sites so the wasm build uses the *global* installed pool — i.e. call `.into_par_iter()`
  WITHOUT `pool.install()` on wasm (global pool), or return `rayon::current_thread_...`. Prefer:
  a thin `with_pool(|| …)` seam that is `pool.install(f)` on native and `f()` (global) on wasm,
  wrapping the three `pool.install` sites (`parallel_workers` ~143, `run_parallel_queue_ref`
  ~732, `run_parallel_fold` ~1040). **Code points:** `parallel.rs` `rayon_pool` + the 3
  `pool.install` sites.
- **Gate:** native suite still green (byte-identical `loft introspect` for a `par` fixture — no
  emit change); `make wasm-mt` builds.

### Step 3 — init the pool in the nodejs harness (retire `LoftThreadPool`)

- `tests/wasm/harness.mjs::initThreaded` currently builds the hand-rolled `LoftThreadPool`
  (parallel.mjs). Replace with wasm-bindgen-rayon's flow: instantiate the module, then
  `await instance.initThreadPool(n)` (its generated worker bootstrap). **Code points:**
  `tests/wasm/harness.mjs` (initThreaded), delete usage of `tests/wasm/parallel.mjs`.
- **Gate:** the Step-0 instrument now reports `distinct_worker_ids ≥ 2` AND the correct value —
  **this is the milestone**: a `par` provably running on multiple wasm workers.

### Step 4 — delete the hand-rolled worker model

- Remove `src/wasm.rs::worker_entry` (the stub), `tests/wasm/worker.mjs`,
  `tests/wasm/parallel.mjs`, and any `LoftThreadPool` references. **Code points:** those files +
  their importers.
- **Gate:** `make wasm-mt` + the threaded suite green; no dangling imports (`node --check`).

## Track 2 — re-prove the memory model under SHARED memory (interleave with Track 1)

This is the correctness spine; do it *as* Steps 2–3 land, not after. On native each worker got a
`clone_for_light_worker` borrow and the address spaces were distinct; under wasm every worker
shares ONE linear memory (the whole Store heap is one `SharedArrayBuffer`), so the copy/borrow
lifetime argument must be re-examined.

### Step M0 — the positive-control race (must fire)

- Port @PLN108's positive-control: a deliberately-racy `par` (two workers write the same store
  slot) that MUST trip the `read_only` write-panic (or a debug checker). **Code points:** a
  `#[cfg(feature = "wasm-threads")]` test mirroring `tests/threading.rs`'s positive control.
- **Gate:** the racy program panics/aborts as designed — proving the clean runs below are
  non-vacuous.

### Step M1 — audit the share model under shared memory

- Read `state/mod.rs::clone_for_light_worker` / `clone_for_worker` / `par_share_for` and
  `THREADING.md:229–231` with the shared-memory lens: (a) `clone_for_light_worker` shares each
  store's `ptr` read-only — still sound when the ptr is into a *shared* SAB and all workers are
  joined before the parent drops? (b) `clone_for_worker`'s freed-slot reinit
  (`Store::new(100)`) — does allocating a fresh store inside shared memory while other workers
  run introduce a data race on the allocator? (c) the 2 MB `par_share_for` heuristic — is the
  copy path even affordable under wasm memory caps, or must wasm always borrow?
- **Code points:** `src/state/mod.rs` (the three fns), `src/parallel.rs` (`par_share_for`,
  `clone_for_light_worker` call sites), `src/database/` (Store allocation).
- **Gate:** the Step-0 instrument stays value-correct AND the M0 race still fires; add a
  wasm-side leak/'`read_only`-panic check as the TSan-analogue.

## Track 3 — the browser (`--target web`) build + deployment contract

Only after Track 1+2 are green on nodejs (the logic is proven; the browser adds transport +
headers, not new correctness).

### Step B1 — a threaded `--target web` bundle

- Add a Makefile target mirroring `wasm-mt` but `--target web` (the RUSTFLAGS
  `+atomics,+bulk-memory,+mutable-globals` are the same), output to `doc/pkg` (gallery) — and
  fold the same into the `loft --html` export pipeline. **Code points:** `Makefile` (new
  `wasm-mt-web` target + the gallery/`--html` build at `Makefile:372`).
- **Gate:** bundle builds; loads in a COOP/COEP dev server without instantiation error.

### Step B2 — JS glue: init + `crossOriginIsolated` detection + sequential fallback

- Before any `par`, the page must `await init(); if (crossOriginIsolated) await
  initThreadPool(navigator.hardwareConcurrency)`. When `!crossOriginIsolated`, **skip pool
  init** — the wasm build then falls back to the sequential path (Track-1 code must keep the
  `not(threading)` fallback reachable at runtime, i.e. an uninitialised pool ⇒ 1 worker, never a
  crash). **Code points:** the gallery/playground loader JS in `doc/` + the `--html` runtime
  shim.
- **Gate:** page with COOP/COEP → pool of N; page without → runs sequential, no error (the
  fallback-never-breaks invariant, arc D).

### Step B3 — the COOP/COEP hosting contract

- The dev server (`make serve`) and the gallery deploy must send
  `Cross-Origin-Opener-Policy: same-origin` + `Cross-Origin-Embedder-Policy: require-corp`.
  Document the contract in `WASM.md` / `BROWSER_INTEROP.md` so routing/game hosts can meet it;
  note the sub-resource (`require-corp`) implication for any cross-origin asset. **Code
  points:** `make serve` target, deploy config, `doc/claude/WASM.md`.
- **Gate:** `crossOriginIsolated === true` in the served page; a routing/game demo runs `par`
  off the main thread.

## Track 4 — verify + measure (graduates as tracks land)

- **E1 off-main-thread:** an in-browser demo where a heavy `par` runs while a
  `requestAnimationFrame` counter keeps ticking (UI not blocked) — the qualitative proof.
- **E2 scaling:** the Step-0 instrument as a benchmark: wall-clock vs `initThreadPool(1..N)`,
  asserting speedup; also vs native and vs sequential-wasm.
- **E3 CI gate:** wire `make wasm-mt` + `node tests/wasm/suite.mjs --threaded` into the CI
  matrix (nodejs only — no browser/headers needed), so the threaded path can't silently rot
  (the same lesson as the bridge-test rot).

## Landing order (each is a small PR-sized commit)

`Step 0` → `Step 1` → `Step 2` (+ `M0`) → `Step 3` (+ `M1`) → **milestone: par threaded on
nodejs, memory-model-proven** → `Step 4` → `B1` → `B2` → `B3` → **milestone: par threaded
in-browser with fallback** → `E1/E2/E3`. Stop-and-revert if any gate regresses the native
suite or the M0 race stops firing.

## Risk register

- **R1 (highest, S-category):** shared-memory data race the native model didn't have — mitigated
  by M0/M1 (positive-control-first).
- **R2:** `wasm-bindgen-rayon`'s global pool doesn't drive loft's `.into_par_iter()` as expected
  — surfaced immediately at Step 3's gate; fallback is the hand-rolled dispatch (reversing the
  step-0 decision), so it's a bounded, early risk.
- **R3:** COOP/COEP unavailable on a target host → the whole SAB path is unusable there;
  mitigated by the never-break sequential fallback (B2) — the app runs, just 1×.
- **R4:** wasm memory caps make the copy path unaffordable → M1 decides borrow-always for wasm.
