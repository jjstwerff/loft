<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# 117 — Browser multi-threading (real Web Worker threads for `par` / `par_fold`)

Tracks [`loft-lang/plans#117`](https://github.com/loft-lang/plans/issues/117) (`@PLN117`).

## Status

**Open — core + memory model + gallery proven end-to-end in-browser (steps 0–4, arcs A/B/C/D,
B1-gallery); `loft --html` export + benchmarks remain.** A loft `par`, run through `compile_and_run`
in a real (headless) browser, **dispatches across multiple Web Worker threads over shared wasm
memory** with correct results (proven for primitive, struct+text, and vector returns) and a
never-break sequential fallback. The load-bearing risk (does
rayon-via-`wasm-bindgen-rayon` actually build, link, and parallelise loft's dispatch?) is
**resolved**. What is proven, landed on `tuxedo-extract-function`, and pushed:

- **Build + link + export** (`src/wasm.rs` re-exports `init_thread_pool`; `make wasm-mt`).
- **Native dispatch seam** (`src/parallel.rs::with_pool`) — wasm uses the global rayon pool,
  native its private pool; behaviourally identical on native (threading suite 47/47).
- **In-browser dispatch:** `distinct_workers=4`, `par_sum=36023`, matching the native
  `--interpret` **and** `--native` reference (`36023`). ~1.8× wall-clock here (interpreter
  overhead dominates 48 elems / 4 workers).
- **Arc-D fallback:** with no `initThreadPool`, the same `par` runs sequentially
  (`distinct_workers=1`), succeeds, and gives the correct value — **it never breaks**.
- **Arc-C memory model:** the read-only-share model holds under shared linear memory —
  allocation-heavy struct+text AND vector-return `par` match the native reference on every
  rep across 4 workers (30-rep race hunt clean); the wasm allocator lock-guards DLMALLOC.
- **Headless gates:** `tests/wasm/par-thread-proof.sh` (dispatch + fallback) and
  `par-memory-proof.sh` (memory model, both return shapes); `coi-server.py` is the COOP/COEP host.

**The toolchain finding (load-bearing).** `wasm-bindgen-rayon` only threads if the wasm memory
is SHARED + IMPORTED with a maximum and lld's synthesized TLS / heap-base globals survive as
exports. This toolchain's rustc does **not** auto-emit those from `+atomics`, so `make wasm-mt`
passes the full link-arg set explicitly (`--shared-memory --max-memory --import-memory
--export=__heap_base --export=__wasm_init_tls/__tls_size/__tls_align/__tls_base`). Drop any one
and the bundle silently builds a **non-shared** memory → workers die at runtime with *"Memory
could not be cloned"*. `Cargo.toml` also enables `wasm-bindgen-rayon`'s `no-bundler` feature so
the worker bootstrap resolves the main module by URL (loft ships static files, no bundler).
`--target web` is mandatory (the old `wasm-mt` target used `--target nodejs`, which cannot drive
the rayon worker bootstrap; **node also has no Web Worker global**, so the browser — not node —
is the proof environment).

**Remaining:**

- **B1 — `loft --html` threaded export (its own arc; design fork).** The gallery half is done;
  the self-contained single-file `--html` export is NOT a quick follow-on. `--html` inlines the
  wasm as **base64 in a non-module `<script>`** (`src/main.rs` ~6929), which is incompatible with
  `wasm-bindgen-rayon`'s worker bootstrap (ESM `--target web` + a separate `workerHelpers.js`
  imported via `import.meta.url`; workers `new Worker(url,{type:'module'})` then re-import the
  main module). Two options, pick under the design skill: **(a)** a custom inlined worker
  bootstrap — each worker instantiates the inlined base64 wasm against the SHARED memory and runs
  the rayon worker loop (self-contained kept; hand-rolled, the case the retired stub was meant
  for); **(b)** switch `--html` to a multi-file ESM bundle for the threaded path (drops the
  single-file property games rely on). Also needs the `--html` rustc invocation to add the
  atomics/build-std/shared-memory flags + run the wasm-bindgen threads transform. Games use little
  `par` today, and the routing consumer can deploy the multi-file `pkg-mt` on a COOP/COEP host —
  so this is lower urgency than it looks.
- **Track 4 E1** (off-main-thread UI-responsive demo) + **E2** (scaling benchmark, par vs threads
  1..N); wiring the headless gates (`par-thread-proof.sh`, `par-memory-proof.sh`) into CI (needs
  chromium on the runner).

Steps 0–4 + arcs A/B/C/D + E3 + B1-gallery are done. This README is the single source of truth
for phase status.

loft's parallel execution otherwise runs **sequentially on the main thread in the browser**: the
default `--target web` bundle is built `--features wasm` (no `threading`), so `par(...)` and
`par_fold` take `parallel.rs`'s non-threading fallback. This plan wires the `wasm-threads` path
(rayon backed by `wasm-bindgen-rayon` = Web Workers over SharedArrayBuffer + wasm atomics) into
the browser build so the whole parallel-dispatch surface runs on real threads, at parity with the
native OS-thread (rayon) model.

## Goal

Browser **games** — loft's core use case — and apps like **routing** need genuine
multi-threading. Make `par(...)` and `par_fold` dispatch across Web Worker threads in the
browser (off the main thread, UI stays responsive, scales with core count), with a clean
fall-back to today's sequential behaviour when the host can't provide cross-origin isolation.

## Effort + design

- **Effort:** L — runtime + build wiring + JS glue + a real deployment constraint (COOP/COEP).
- **Value:** G (goal-enabling: browser games / multiplayer / routing all depend on it).
- **Design:** ready — the step-by-step plan (small gated steps + exact code points) is in
  [`DESIGN.md`](DESIGN.md); this is wiring + re-proving the memory model under shared memory,
  not new invention.

## Why it exists

`par` is loft's data-parallel primitive; on `--native` it runs OS threads via rayon
(`std::thread::spawn`, `thread::scope`). In the browser it currently does **nothing parallel** —
`src/parallel.rs` (lines 7–9): *"When `threading` is disabled (e.g. under WASM), the loop body
runs sequentially in the caller's thread — same results, no parallelism."* For a routing solver
or a game's per-entity update, that means the whole workload runs on the main thread and freezes
the page while it does. The infrastructure to fix it already exists but is unwired:

- `Cargo.toml`: `wasm-threads = ["wasm", "threading", "dep:wasm-bindgen-rayon"]`.
- `Makefile:951`: a **nodejs** multi-thread bundle (`tests/wasm/pkg-mt/`) is built with it.
- `src/wasm.rs::worker_entry`: a **no-op stub** (W1.18-2 TODO) — an older hand-rolled-worker
  design; the intended model is the rayon/`wasm-bindgen-rayon` scheduler (`parallel.rs:140`,
  *"so the two scheduler stories converge"*).

So the browser (`--target web`) build just never opts in.

## The two threading models this must cover

1. **`par(...)` for-loop** — `run_parallel_*` in `src/state/mod.rs` / `src/parallel.rs`
   (`.into_par_iter()` under `#[cfg(feature = "threading")]`), including the @PLN108
   read-only-store-share optimisation.
2. **`par_fold` / `parallel_fold`** — the parallel-fold builtin (`parse_par_fold`,
   `parallel::run_parallel_fold`).

Both are rayon-backed, so both ride the same wasm-pool wiring. **Coroutines** (COROUTINE.md) are
cooperative single-thread state machines — they are *not* threaded and stay as-is, but must keep
working correctly on the main thread alongside the worker pool.

## Sub-arcs

| Item | Concern | Status |
|---|---|---|
| **A** — threaded `--target web` bundle | build the browser bundle with `--features wasm-threads`; JS glue inits the pool (`await init(); await initThreadPool(navigator.hardwareConcurrency)`) before any `par` | **Build + gallery DONE.** `make wasm-mt` (full link-arg set, `no-bundler`); pool proven to initialise in-browser (`pool_init=ok`, N workers). **B1 gallery:** `make gallery-mt` (→ `doc/pkg-mt`, no clobber of the committed single-threaded gallery); `playground.html`/`gallery-run.html` call `initThreadPool` when `crossOriginIsolated` (inert on the single-threaded bundle — proven both ways); `make serve` sends COOP/COEP. **`loft --html` fold-in pending** — see below |
| **B** — dispatch over the wasm rayon pool | route `run_parallel_*` and `par_fold` over the `wasm-bindgen-rayon` pool; retire the `worker_entry` stub so there is ONE scheduler | **Dispatch DONE + proven** (`with_pool` seam; in-browser `distinct_workers=4`, value matches native). Retiring the hand-rolled stub = step 4, pending |
| **C** — memory model under SHARED memory | re-prove the @PLN108 read-only-share model (`clone_for_light_worker`, borrowed parent stores, `read_only` write-panic) now that the Store heap is a *shared* `SharedArrayBuffer` across workers, under atomics | **DONE + proven.** Invariant (worker reads shared read-only parent, writes only own scratch) holds across 5 verified re-assertion sites — C93 static, the release-active read-only `assert!`, the read-only borrow, join-before-drop, and the **lock-guarded wasm DLMALLOC** (concurrent worker alloc is serialized). Empirical: `par-memory-proof.{html,sh}` runs allocation-heavy struct+text AND vector-return `par` across 4 Web Workers, every rep == native ref, real ≥4-worker concurrency, 30-rep race hunt clean. `par_share_for`/copy-path is gone (@PLN108 left ONE borrow path) |
| **D** — cross-origin isolation + fallback | COOP/COEP hosting contract; runtime `crossOriginIsolated` detection; **graceful fallback to sequential** when unavailable | **DONE + proven** — `coi-server.py` sends COOP/COEP; without `initThreadPool` the `par` runs sequentially (`distinct_workers=1`) and never crashes. B3 (document the contract in WASM.md for real hosts) pending |
| **E** — verify + measure | in-browser proof a `par` runs OFF the main thread + scales; an automated gate; benchmark vs native + vs sequential-wasm | **E3 DONE** (`par-thread-proof.sh` headless gate). E1 (off-main-thread UI-responsive demo) + E2 (scaling benchmark) pending |

## Phase ordering

1. **A** first — a `--target web` bundle that builds with `wasm-threads` and whose JS glue
   spins up a worker pool is the prerequisite everything else is verified against. Prove the
   pool initialises (`crossOriginIsolated === true`, N workers) before touching dispatch.
2. **C alongside B** — the memory model must be re-proven *as* dispatch is wired, not after:
   shared linear memory changes the copy-vs-borrow assumptions, and a data race here is silent
   corruption (an **S**-category hazard). Build the falsification instrument (a positive-control
   race that MUST fire) first, like @PLN108 did on native.
3. **B** — wire the pool once A+C hold; the `.into_par_iter()` paths should light up under the
   pool with little change, but `clone_for_worker`'s freed-slot reinit and the locked-store
   asserts need checking under atomics.
4. **D** runs throughout (the fallback path must exist from A so nothing ever hard-breaks) and
   is the deployment gate for real hosts.
5. **E** graduates as each arc lands; the pkg-mt harness is the CI-able proxy for the browser.

## Open questions

1. **rayon-via-wasm-bindgen-rayon vs the hand-rolled `worker_entry`.** Confirm the
   `wasm-bindgen-rayon` pool actually drives `.into_par_iter()` in loft's build (it should — it
   installs a global rayon pool) and delete `worker_entry` rather than finishing it. If a
   loft-specific reason blocks rayon-on-wasm, the hand-rolled dispatch is the fallback design.
2. **@PLN108 borrow model under shared memory.** On native, the read-only-share borrows parent
   stores because the dispatcher joins all workers before the parent drops. Under a shared
   `SharedArrayBuffer`, is the same lifetime argument sound, or does shared memory + atomics
   force back to the copy path (and if so, is the copy even affordable given wasm memory
   limits)? This is the load-bearing correctness question.
3. **Thread count + pool lifecycle.** One pool per page (init once) vs per-`par`; how
   `navigator.hardwareConcurrency` maps to the `threads` argument of `par(b=f(a), N)`; what
   happens when a game wants a persistent worker set across frames.
4. **Fallback UX.** When `crossOriginIsolated` is false, run sequential — but should it *warn*
   (a page that expected parallelism silently runs 1×), and where (console vs a loft-level hook)?

## Cross-arc dependencies

- **[@PLN108 read-only store sharing](../108-share-read-only-stores/README.md)** — the memory
  model arc C must re-prove under shared wasm memory; its native ASan/TSan gate is the template
  for the wasm race-detection gate.
- **THREADING.md** — the `par` / `par_fold` dispatch contract (worker rules, `clone_for_worker`,
  `run_parallel_*`); reference home, updated as arcs land.
- **WASM.md / BROWSER_INTEROP.md** — the browser build + host-interaction surface; the COOP/COEP
  contract (arc D) documents here.
- **`tests/wasm/` harness** — `pkg-mt` build (Makefile) + `suite.mjs --threaded`; the automated
  proxy for the in-browser gate.
- **Coroutines (COROUTINE.md)** — not threaded, but must keep working on the main thread; a
  regression check, not an arc.

## See also

- `src/parallel.rs` (dispatch + the threading/non-threading split), `src/state/mod.rs`
  (`run_parallel_*`, `clone_for_worker`), `src/wasm.rs` (`worker_entry` stub).
- `Cargo.toml` (`wasm-threads` feature), `Makefile` (`--target web` gallery build + `pkg-mt`).
- [`loft-lang/plans#117`](https://github.com/loft-lang/plans/issues/117) — the tracking issue.
