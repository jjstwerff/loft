<!-- SPDX-License-Identifier: LGPL-3.0-or-later -->
# @PLN105 — layout-aware zero-copy FFI delivery (`deliver` / the loft→JS binary bridge)

> **Status — SHIPPED (language side) 2026-07-16.** All language-side phases (0–3 + the falsifier
> tail) are complete; the durable reference now lives in **[BROWSER_INTEROP.md § The binary bridge —
> `deliver`/`expose`](../BROWSER_INTEROP.md#the-binary-bridge--deliver--expose-zero-copy-layout-driven)**.
> This file is the closure record: what shipped, the decisions, the falsifier suite.
> **Issue:** [loft-lang/plans#105](https://github.com/loft-lang/plans/issues/105) — closed on merge
> via `Closes @PLN105`. **Phase 4 (routing consumer acceptance) is the one open item — owned by the
> routing agent, in `../routing`, not language-side (see below).**

---

## What it is (one line)

JS reads a whole loft value in wasm linear memory — record / vector / enum / nested to any depth —
with no serialization and no copy, driven only by a self-describing layout descriptor loft delivers
alongside the value; keyed collections (hash/index/sorted/radix) are pre-flattened at delivery, so JS
only ever interprets `{scalar, text, record, vector, ref, enum}`. **Full reference:**
[BROWSER_INTEROP.md § The binary bridge](../BROWSER_INTEROP.md#the-binary-bridge--deliver--expose-zero-copy-layout-driven).

## What shipped (phase log)

- **Phase 0 — descriptor emitter.** `LayoutDesc` + `read_via_descriptor` (`src/database/descriptor.rs`):
  the read-only twin of `read_data`, driven by the descriptor. Three independent oracles agree
  byte-for-byte; `Iterated`/`Ref`/`ChildRec` refused (not panicked). Pure loft-side, both backends.
- **Phase 1 — `deliver` stdlib + lowering + loopback host.** `deliver`/`OpDeliver`; `deliver_loopback`
  reconstructs == original (interpret == native). SYNCHRONOUS → not in the asyncify set.
- **Phase 2 — the generic JS reader.** `readLoftValue` (`doc/loft-deliver.js`), the twin of
  `read_via_descriptor`: scalars, narrow ints, text (interned inline), record, enum, vector (scalar
  **fast lane** = zero-copy typed-array view), by-ref array. wasm `loft_host_deliver` import; browser
  `deliver_browser` computes the raw linear-mem address (`store.ptr` IS a wasm address). Parity gate
  interpret == native == `--html` holds end-to-end (`tests/deliver_wasm.rs` + `tools/deliver_repro.mjs`).
  - **P2.d** — reader inlined into BOTH `--html` page shells (`main.rs` embeds `doc/loft-deliver.js`),
    so a real page auto-reconstructs; `expose` stashes a per-frame re-reader closure.
  - **P2.e** — memory.grow-safety cell: grow detaches the `ArrayBuffer` mid-flow, the reader
    re-derives its view (`deliver_survives_memory_grow_after_expose`).
  - **P2.f** — corpus shapes: **narrow ints** (fixed a real bug — reader ignored the `start` offset +
    read shorts signed) and **value enum** (fixed a real bug — 1-based disc, off-by-one); by-ref array
    (unreachable from loft source — synthetic guard `tools/reader_array_unit.mjs`).
- **Phase 3 — keyed collections, PRE-FLATTENED.** hash/radix/index → materialise to a key-ordered
  scratch array (`build_*_sorted_vec`) → `FlatArray`; `sorted` → in-place `Vector` (already
  key-ordered); nested + deep (sub-struct) fields; multi-instance via a `flat` redirect map keyed by
  `(rec,pos)` so one type-shared node serves every instance. `#`-prefixed synthetic tree fields
  skipped. JS reads them layout-blind via the array path.
- **`expose` / `release` + cross-frame.** Long-lived deliver: pins the store (`lock_store`) across
  frames; `release` unpins. The cross-frame read survives an asyncify yield
  (`deliver_expose_survives_cross_frame_yield_in_js`, `tools/deliver_crossframe.mjs`) — the only
  headless suspend is `loft_host_http_get` (`store_load_url_trusted`); `yield_frame()` is
  interpreter-only.
- **Zero-copy O(1) perf falsifier** (probe #3 — the feature's raison d'être). Proven STRUCTURALLY:
  loft-side node is a plain `vector` (no materialisation), JS-side value is a typed-array VIEW aliasing
  `mem.buffer`; corroborated by a same-machine view-vs-O(n)-scan ratio (~1000×).
  `deliver_scalar_vector_is_zero_copy_o1_in_js`. **Nuance:** O(1) is the inline fast-lane path only —
  keyed collections are O(n) by the pre-flatten design, by intent.

## Decisions (the pivots from the original sketch)

- **Keyed collections pre-flatten, NOT JS cursors** (decided 2026-07-15). `loft_start` makes `Stores`
  a local, so JS-called `loft_iter_*` exports can't reach it without a risky global-store change;
  materialise-at-delivery reuses the array reader unchanged.
- **Descriptor passed INLINE** (`descPtr,descLen` with the handle), not a separate `loft_layout_desc`
  export — the deliver body already holds the `Stores`; avoids a wasm export reaching global state.
- **`deliver` is synchronous** (borrow ends on return) → deliberately NOT in the asyncify set;
  `expose` carries the long-lived case via store pinning.
- **The reader is a VALUE reconstructor, the Rust twin a byte oracle.** `read_via_descriptor` emits
  packed parity bytes (narrow ints, raw 1-based enum) as a stable cross-backend fingerprint;
  `readLoftValue` reconstructs the true value — they are twins in traversal, not scalar representation.

## Tests — the definition of done

| # | Gate | Phase | Status |
|---|---|---|---|
| 1 | Descriptor round-trip (`read_via_descriptor` == `read_data`, `--interpret`) | 0 | ✅ |
| 2 | `Iterated` positive control (keyed field → `Iterated`, no panic) | 0 | ✅ |
| 3 | Boundary loopback parity (reconstruct == original, interpret == native) | 1 | ✅ |
| 4 | Headless browser parity (interpret == native == `--html`, byte-identical) | 2 | ✅ |
| 5 | memory.grow safety (grow mid-read, reader re-derives, no detach throw) | 2 | ✅ |
| 6 | Cursor/flatten reconstruction (keyed multiset == interpreter iteration) | 3 | ✅ |
| 7 | Zero-copy O(1) (inline `vector<scalar>` view, not a copy) | 2/3 | ✅ |
| 8 | Routing acceptance (`view`/`match` == native text path, exact fixed-point) | 4 | ⏳ consumer |

`tests/deliver_wasm.rs` 17/17; `tools/deliver_{repro,crossframe,perf}.mjs` + `reader_array_unit.mjs`.

## Phase 4 — routing migration (the one open item, consumer-owned)

Language-side prerequisites are all done; what remains is genuinely consumer work in `../routing`
(owned by that agent): swap routing's `view`/`match` text-serialize for `deliver(...)` + the canvas
`parseFloat`-per-coord renderer for the reader + typed-array lanes, with the coordinate round-trip
acceptance (`i32/1e7` == the text path). Optional tails from the consumer spec
(`../routing/docs/loft-binary-bridge.md`): the games GPU-upload path (`gl.bufferData` straight from a
delivered interleaved vertex buffer) and a `--native-wasm` framed-stdout binary sink for headless
wasmtime testing. None are language-side; @PLN105 closes on the merge of the language work.

## Non-goals

Not a serialization *format* (each consumer layers its own on top). Not nested pointer-graph
transfer (flat elements). Not a shared-memory GC contract beyond the borrow window. Only pays for
large payloads — text stays the default control channel.
