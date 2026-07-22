<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# 117 — Browser multi-threading (real Web Worker threads for `par` / `par_fold`)

Tracks [`loft-lang/plans#117`](https://github.com/loft-lang/plans/issues/117) (`@PLN117`).

## Status

**Open — design ready, no implementation yet.** loft's parallel execution runs
**sequentially on the main thread in the browser**: the `--target web` bundle is built
`--features wasm` (no `threading`), so `par(...)` for-loops and `par_fold` take `parallel.rs`'s
non-threading fallback — no parallelism, and a heavy `par` blocks the UI. This plan wires the
already-scaffolded `wasm-threads` path (rayon backed by `wasm-bindgen-rayon` = Web Workers over
SharedArrayBuffer + wasm atomics) into the browser build so the whole parallel-dispatch surface
runs on real threads, at parity with the native OS-thread (rayon) model. This README is the
single source of truth for phase status.

## Goal

Browser **games** — loft's core use case — and apps like **routing** need genuine
multi-threading. Make `par(...)` and `par_fold` dispatch across Web Worker threads in the
browser (off the main thread, UI stays responsive, scales with core count), with a clean
fall-back to today's sequential behaviour when the host can't provide cross-origin isolation.

## Effort + design

- **Effort:** L — runtime + build wiring + JS glue + a real deployment constraint (COOP/COEP).
- **Value:** G (goal-enabling: browser games / multiplayer / routing all depend on it).
- **Design:** ready (the pieces exist; this is wiring + re-proving the memory model under
  shared memory, not new invention).

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
| **A** — threaded `--target web` bundle | build the browser + `--html`-export bundle with `--features wasm-threads`; JS glue inits the pool (`await init(); await initThreadPool(navigator.hardwareConcurrency)`) before any `par`; gallery/playground + `loft --html` paths | Open |
| **B** — dispatch over the wasm rayon pool | route `run_parallel_*` and `par_fold` over the `wasm-bindgen-rayon` pool; retire (or finish) the `worker_entry` stub so there is ONE scheduler (native + wasm both rayon) | Open |
| **C** — memory model under SHARED memory | re-prove the @PLN108 read-only-share model (`clone_for_light_worker`, borrowed parent stores, `read_only` write-panic, the 2 MB `par_share_for` heuristic, `clone_for_worker` freed-slot reinit) now that the Store heap is a *shared* `SharedArrayBuffer` across workers, under atomics; the wasm equivalent of the ASan/TSan-clean gate | Open |
| **D** — cross-origin isolation + fallback | COOP/COEP hosting contract (`Cross-Origin-Opener-Policy: same-origin` + `Cross-Origin-Embedder-Policy: require-corp`); runtime detection of `crossOriginIsolated`; **graceful fallback to sequential** when unavailable (a page must still run, just single-threaded) | Open |
| **E** — verify + measure | in-browser proof a `par` runs OFF the main thread (UI responsive) + scales; `tests/wasm/suite.mjs --threaded` (pkg-mt) as the automated gate; routing/game-shaped benchmark vs native + vs sequential-wasm | Open |

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
