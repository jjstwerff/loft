<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Phase 8 — Browser parallel par via Web Workers

**Status: open**

## Goal

After phase 1 lands, native + interpreter par are real-parallel.
The browser WASM path (`doc/pkg/`) still runs sequentially.  Phase
8 brings real 4-thread parallelism to the browser via Web Workers,
using `wasm-bindgen-rayon` to plug into the same `Stitch` runtime
that phase 1 builds.

This is **vital, not deferred**: a browser-only loft program with
a 4-thread `par(...)` call must actually use 4 cores in the
browser.  Anything else makes the browser the second-class target,
contradicting plan-06's "everything is a store" uniform pipeline.

## Why this is plan-06 scope, not 1.1+

The previous roadmap entry (W1.14 — WASM Tier 2: Web Worker pool)
sat in 1.1+ as VH effort.  That placement assumed the typed-par
runtime would land first and W1.14 would adapt to it later.

Plan-06 inverts the priority: the typed-par redesign IS where the
browser-parallel story lands, because:

1. The per-worker output Store concept (phase 1) maps cleanly
   onto Web Worker postMessage + transferred ArrayBuffer.
2. The `Stitch` policy enum (phase 3) parameterises native and
   browser identically — Concat / Discard / Reduce / Queue all
   work the same regardless of scheduler.
3. The user surface (the fused for-loop + par + par_fold) must
   work identically on both targets — split delivery is hostile
   to users who write `--html` programs and expect parallelism.

Effort goes from VH (rebuild the runtime later) to **MH** (extend
the now-typed runtime with one more scheduler variant).

## Architecture

```
                   ┌─────────────────┐
                   │  Stitch policy  │   ← phase 3
                   │  (Concat | …)   │
                   └────────┬────────┘
                            │
     ┌──────────────────────┼──────────────────────┐
     │                      │                      │
┌────▼─────┐         ┌──────▼──────┐         ┌─────▼──────┐
│ Native   │         │ Interpreter │         │ Browser    │
│ thread   │         │ thread      │         │ Web Worker │
│ ::scope  │         │ ::scope     │         │   pool     │
│ (phase 1)│         │ (phase 1)   │         │ (phase 8)  │
└──────────┘         └─────────────┘         └────────────┘
```

Same `Stitch` policy, same per-worker output Store, three
schedulers.

### Web Worker pool via `wasm-bindgen-rayon`

The Cargo feature `wasm-threads` already gates this in
`Cargo.toml` (`["wasm", "threading", "dep:wasm-bindgen-rayon"]`).
Today the feature is opt-in and unused in the gallery deploy.
Phase 8 makes it the **default** for browser deploys.

`wasm-bindgen-rayon` requirements:
- **SharedArrayBuffer** — needs cross-origin isolation headers
  (`Cross-Origin-Opener-Policy: same-origin` +
  `Cross-Origin-Embedder-Policy: require-corp`) on the serving
  page.
- **wasm-pack build with `--target web`** + a small JS shim that
  initialises the worker pool before user code runs.
- **GitHub Pages compatibility**: COOP/COEP headers can be set
  via `<meta http-equiv>` tags or a `_headers` file (Netlify
  syntax).  GitHub Pages needs the meta-tag approach.

### Per-worker output Stores in the browser

The same per-worker output-slot concept from phase 1 applies —
each Web Worker has an output slot in its `WorkerStores.allocations`,
writes via the standard `OpSet*` opcodes, and the parent extracts
the slot's Store after join.

**Cross-thread sharing requires a SAB-backed Store allocator —
this is a hard prerequisite, not a free side-effect.**  Today's
`Store::new(size)` calls `std::alloc::alloc_zeroed` against the
process global allocator; the resulting buffer lives in the main
thread's linear memory only.  Web Workers run in separate WASM
instances with separate linear memories — they **cannot see**
main-thread allocations regardless of pointer values.

Phase 8's prerequisite (sub-phase **8a'**, must land before 8a):

- Add a `Store::new_shared(size)` constructor that allocates from
  a `WebAssembly.Memory({shared: true})` SAB pool when the
  `wasm-threads` feature is enabled; falls back to the system
  allocator otherwise.
- When `wasm-threads` is enabled, `Stores::database` and
  `WorkerStores::add_output_slot` route through `new_shared`.
- The SAB pool's growth strategy mirrors the system allocator's
  (page-aligned blocks, no fragmentation guarantees beyond what
  the JS engine provides).
- A runtime feature-detection check on parent-store creation
  asserts the SAB-backing succeeded; failure falls back to the
  sequential WASM path with a `console.warn`.

**Implication**: existing parent state (the const store, user-
allocated parent data) accumulated **before** the first
`par(...)` call must already be SAB-backed.  The decision is
made at parent-store-allocation time, not par-call time.  A
parent that allocated stores via `new` (system allocator) and
then enables `wasm-threads` mid-program cannot make those stores
visible to workers without copying — and copying defeats the
purpose.

**For workloads with hundreds of MB of parent state**: the SAB
allocator must work end-to-end from the parent's first
allocation; this includes the loft const store (often the
largest single allocation in any non-trivial program).  The
existing CONST_STORE initialisation path in `State::new` must
route through `new_shared` under the `wasm-threads` feature.

After workers finish, the parent reads from each worker's output
slot via the **same rebase walk from phase 2** — see DESIGN.md
D13a.  The rebase is not optional: a Web Worker's output Store
contains DbRefs whose `store_nr` is **worker-local** in the
worker's runtime instance.  After `postMessage` transfer, those
`store_nr` bytes name a worker-local store that doesn't exist
in the parent's store table.  The parent must run the rebase walk
to rewrite each `store_nr` field to the parent-side store_nr
returned by `Stores::adopt_store`.

For primitive-only output Stores (no DbRef fields, per D13b), the
rebase walk is a no-op and the SAB transfer is zero-cost — parent
reads the SAB-backed buffer directly.

### `postMessage` is the join, rebase is the stitch

Native: `thread::scope` join is implicit when the closure exits;
phase-2 rebase walks the per-worker output Stores.

Browser: each Web Worker posts a "done" message + Transferable
ArrayBuffer holding its output slot's SAB-backed buffer (and any
intermediate worker stores) when it finishes.  The parent collects
all N messages, reconstitutes each buffer as a `Store` and calls
`Stores::adopt_store(store) -> u16` to install it, populating the
rebase map with `(worker_id, worker_local_store_nr) →
parent_store_nr`, then runs the rebase walk per DESIGN.md D13a.

The buffer transfer is zero-copy because SAB is `Transferable`.
The rebase walk's cost is the same as native (per-DbRef-field
rewrites scoped to `data::owned_elements`).

## Per-commit landing plan

### 8a — `wasm-bindgen-rayon` smoke

- Add `wasm-threads` to the `wasm-pack` build's default features
  for the gallery / playground bundle.
- Add the JS shim that initialises the worker pool on load.
- Smoke: a trivial loft program with `par([1,2,3,4], identity, 4)`
  runs and produces the right result in the browser.
- Bench: `bench/11_par` under wasm — first non-`-` number in the
  loft-wasm column.

### 8b — Web Worker pool wired to `Stitch::Concat`

- Replace the sequential `run_parallel_browser_concat` from phase
  1's WASM fallback with a real `wasm-bindgen-rayon`
  `par_iter().map(...).collect()` shape.
- Per-worker output Stores allocated as SAB-backed buffers.
- Parent rebase pass (phase 2) handles the join — explicitly
  invoke `rebase_walk_record` from `src/parallel.rs` after
  `postMessage`-receive, before exposing the result vector to
  user code.  Per DESIGN.md D13a, this is **not optional**:
  worker-local `store_nr` values must be rewritten to parent-side
  ones for any DbRef field in the output.
- For primitive-only outputs (D13b): skip the rebase walk; SAB
  transfer alone is sufficient.  Detected by inspecting the
  worker fn's return `Type` at codegen time.

### 8c — Other Stitch policies

- Discard: workers run, drop their output stores.  Trivial.
- Reduce: workers compute partials, parent combines.  Maps
  cleanly to rayon's `reduce`.
- Queue: bounded SAB-backed queue; producer Web Workers push,
  parent body pops.  Most complex; requires SharedArrayBuffer
  atomics.

### 8d — COOP/COEP deployment + cache coherence

- `doc/gallery.html` + `doc/playground.html` add the meta-tag
  COOP/COEP headers.
- `doc/brick-buster.html` (the `--html` self-contained build)
  same.
- CI's `make gallery` step verifies the deployed pages serve
  with the right headers (probe via `node` + a fetch test).
- **HTML/WASM version pinning** — per DESIGN.md D13c, the
  `<script src=…>` reference for the WASM module is regenerated
  alongside the WASM bundle so any HTML/WASM pair is mutually
  consistent.  `wasm-pack` already emits hashed filenames
  (`loft_wasm_bg.<hash>.wasm`); `make gallery` updates the HTML
  to reference the freshly-built hash.
- CI assertion (in `make gallery`): after build,
  `grep loft_wasm_bg gallery.html | grep -o 'loft_wasm_bg\.[a-f0-9]*\.wasm'`
  equals the file actually shipped to `doc/pkg/`.  Mismatch fails
  the build.
- **Runtime fallback** — JS shim checks `crossOriginIsolated`
  before initialising the worker pool.  If false (cached HTML
  pre-COOP/COEP, embedded webview, older Safari), the shim
  falls back to the sequential WASM path with a `console.warn`.
  No crash, no silent wrong answer.

### 8e — Bench + doc

- `bench/11_par`'s `loft-wasm` column reports a real number
  (expected: 5–15 ms, faster than today's `-` and slower than
  loft-native because of postMessage overhead).
- THREADING.md baseline section gets a 5th column.
- CHANGELOG entry for the user-facing story: "Brick Buster + the
  gallery now use 4-thread parallelism in the browser".

## Loft-side prerequisites

- **Phase 1 must land first** — output Store concept underpins
  everything else.
- **Phase 2 (stitch via rebase)** lets the parent read from per-
  worker stores without per-byte copy; matters more in the
  browser where postMessage transfers benefit from zero-copy SAB
  transfer.
- **Phase 3 (one polymorphic native fn)** unifies Native /
  Interpreter / Browser dispatch.

## Acceptance criteria

- `bench/11_par`'s loft-wasm column reports a real number
  (~5–15 ms expected on the bench host's browser; matches or
  beats the loft-interp 44 ms today).
- `make gallery` produces a wasm bundle that runs Brick Buster
  with measurably better frame times when par-using paths run
  (e.g. ball physics if it gets a parallel update).
- `tests/threading_chars.rs` runs under WASM-with-threads via
  the test harness (a new `loft-wasm` cargo nextest profile or
  similar) — same correctness as native + interpreter.
- COOP/COEP headers verified on the deployed gallery.
- CHANGELOG entry framing: "loft programs are now parallel in
  the browser, not just on desktop".

## Risks

| Risk | Mitigation |
|---|---|
| GitHub Pages doesn't support COOP/COEP via HTTP headers | Use the `<meta http-equiv>` approach.  Verified to work for SharedArrayBuffer in Chrome / Firefox / Safari ≥ 2022. |
| Cached pre-COOP/COEP HTML loads with new WASM (silent SAB failure) | Per DESIGN.md D13c, hashed WASM filenames + JS-shim runtime check on `crossOriginIsolated` give defence in depth — old HTML references old WASM (still works); new HTML references new WASM (works); mismatch falls back to sequential with a console warning instead of crashing. |
| Forgetting to invoke the rebase walk after `postMessage` (would corrupt DbRefs in worker results) | The browser dispatcher's `adopt_browser_worker_output(buffers, worker_id)` helper combines `adopt_store` per buffer + rebase walk in one call; no path adopts a worker output without running the walk.  Asserted by `tests/issues.rs::par_phase8_browser_dbref_rebased` — a fixture worker that returns `vector<Reference<T>>` from a Web Worker; assert every result DbRef's `store_nr` resolves to a valid parent store after stitch. |
| `wasm-bindgen-rayon` build takes > 5 min in CI | Cache the build via the existing `actions/cache` step in `.github/workflows/release.yml`. |
| Some browsers (older Safari, embedded webviews) lack SAB support | Fall back to sequential gracefully (the WASM minimal-feature path).  Detected at runtime via `crossOriginIsolated` check; user code sees identical results, just slower. |
| Worker pool startup overhead on first par call | Initialise the pool eagerly when the WASM module loads, not on first par.  ~5 ms one-time cost amortised over the program's lifetime. |
| postMessage overhead per call dominates short workloads | Document: parallelism is worthwhile for workloads > ~1 ms total compute.  Below that, the user can use the sequential fallback explicitly (or just accept the overhead). |

## Out of scope

- Worker pool reuse across `par(...)` calls — desirable
  optimisation, deferred to a follow-up.
- Atomics-based work-stealing scheduler — the rayon backend is
  enough for plan-06; advanced scheduling is post-1.0.
- Cross-origin SharedArrayBuffer scenarios beyond GitHub Pages
  (e.g. Cloudflare Pages, Netlify) — the COOP/COEP headers are
  the same; the deployment glue is platform-specific and can be
  documented as a follow-up.

## Cross-references

- [README.md](README.md) — plan-06 ladder, phase 8 added.
- [DESIGN.md § D6](DESIGN.md) — WASM threading: parallel by
  default; the table this phase implements.
- [01-output-store.md](01-output-store.md) — phase 1 per-worker
  output Stores; phase 8 reuses the same shape.
- [03-one-native-fn.md](03-one-native-fn.md) — phase 3's
  `Stitch` enum parameterises this.
- ROADMAP.md — W1.14 retired (folded into this phase).
- `Cargo.toml` features `wasm`, `wasm-threads` —
  `wasm-bindgen-rayon` dependency.
- `wasm-pack` documentation for `--target web` + worker pool
  initialisation.
