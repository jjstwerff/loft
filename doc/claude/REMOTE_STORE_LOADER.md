<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->
# Remote working-set store loader — design (#522)

**Tracker:** [loft-lang/loft#522](https://github.com/loft-lang/loft/issues/522). Pairs with #517
(HTTP stack) and rides the @PLN97 layout contract ([formal/layout.md](formal/layout.md)).
Status: **design** — not yet a filed plan.

## The one invariant (the testable claim)

> After `store_load_keys` / `store_load_range`, for **every query whose data lies within the
> requested working set**, the local store returns results **byte-identical** to the same query
> against the fully-loaded remote store — while **fetching ≪ the file size** (only the touched
> pages). The local store is afterward a **normal, self-contained heap store** (no residual
> network; iterable; leak-clean).

Everything below is in service of that invariant, and every step's acceptance test asserts a
face of it (identical result **and** bytes-fetched ≪ file **and** leak-clean).

## The second invariant — ZERO cost to non-users (non-regression)

> A program (or build) that never uses `store_load_*` pays **nothing** for this feature — neither
> at **runtime** (the normal `Store::addr` / `find` / read path is unchanged machine code, or
> provably within measurement noise) nor at **build time** (a plain build does not compile the
> HTTP/remote machinery or its dependencies).

This is co-equal with the first invariant and it **decides the architecture** (below): a design
that taxes the hot read path or drags an HTTP dependency into every build is wrong even if it's
functionally correct. Two concrete guards:

- **Runtime — the hot path stays untouched.** `Store::addr` / `get_u32_raw` are the interpreter's
  hottest reads (every field/element access). The paged reader must NOT add a per-read branch there
  (see the `(b1)/(b2)` fork). Verified by a benchmark, not asserted (below).
- **Build — the remote machinery is feature-gated.** Phase 1 (heap store-load) is
  dependency-free and can be always-on; the paged reader + HTTP (phases 2–5) live behind a cargo
  feature (e.g. `remote-store`) so a normal build pulls in **no new dependency** and compiles no
  extra code. `--html`/wasip2 builds that want it opt in.

## Why it is sound: a store file IS its serialization

A store file is a byte-exact image of the arena, self-describing and portable native↔wasm
(`allocation.rs:2117` — *"the on-disk record layout is byte-identical to the in-memory one"*;
formalised in [layout.md](formal/layout.md): record 0 = header, record 1 = `PRIMARY` root,
per-record `i32` size word (words; negative = free), word-based addressing `byte = rec·8`). So
there is **nothing to decode** — reading a record is copying its bytes; an internal reference is a
word offset into the same image. That is exactly what makes range reads possible.

## The load-bearing design decision — how reads virtualize (probe this FIRST)

Today every read goes through `Store::addr<T>(rec, fld)` → `*(ptr + rec·8 + fld)`, where `ptr` is
one contiguous region (mmap or heap). The remote reader has no such region — bytes arrive per
page. This is *the* new store-engine piece, and it must be **probed before it is built**.

Three options:

| option | how | works on |
|---|---|---|
| **A — virtual arena** | allocate a file-size arena; fetch a page into it on first touch; `addr` unchanged (demand-paged anonymous memory = only resident pages cost RAM) | **native only** (overcommit); NOT wasm (linear memory is committed) |
| **B — page cache + `resolve`** | a sparse page table (page-idx → 64 KiB buf, LRU); reads route through `resolve(off, len) → &[u8]` that fetches on miss; a read spanning a page boundary is coalesced into a temp buffer | **all targets** (native, wasip2, `--html`) |
| **hybrid** | B everywhere; A as a native fast path later if profiling demands | — |

**Recommendation: B** (portable; wasm is a hard requirement). The paged reader is **read-only and
transient** — used only to resolve + copy-out; the *result* is a normal heap store — so it needs a
`resolve` primitive + the index traversal, **not** the full mutable `Store` API.

**The reuse-vs-reimplement fork** (the second half of the decision): the elegance #522 wants is
"run the *normal* `find`/range against the remote reader." The existing traversal uses
`addr`/`get_u32_raw` on a `Store`. So either **(b1)** add a `Backing::Paged` variant to `Store` so
`addr`/`get_u32_raw` route through `resolve` (reuses the traversal, but adds a **residency branch to
the hottest read path**), or **(b2)** a **separate `PagedReader` type** with its own
`find`/range over a `&ByteReader` (more code, but the normal `Store` path is **byte-for-byte
untouched**).

**The second invariant decides this: prefer (b2).** A residency branch in `addr` taxes *every*
program's every read, violating "zero cost to non-users" — the elegance of reuse is not worth a
global slowdown. (b1) is admissible ONLY if P0's benchmark proves the branch is free for
non-paged stores (e.g. the optimiser hoists/removes it, or the paged path is a distinct read
function the enum dispatch selects once, not per-read). Default to (b2) — a separate reader keeps
the hot path, and the build, clean.

**Falsification probe P0 (do before any phase):** on a REAL store file written by `bind_path`,
serve its bytes through a `resolve(off,len)` that logs every range touched, and run a single key
`find` over it. **Pass = the found `rec` is correct AND the logged ranges are a handful of pages,
not the whole file.** This proves both that the traversal works over `resolve` and that it fetches
little — the two claims the whole feature rests on. If P0 needs the full `Store` API to work,
option (b1) is forced; if a thin `find`-over-reader suffices, (b2) is cleaner.

## Architecture (two layers + the gate)

```
                       ┌─────────────────────────────────────────────┐
 url ──▶ PagedReader ──│ resolve(off,len): page table + fetch-on-miss │──▶ page provider
         (read-only,   └─────────────────────────────────────────────┘     (local file | #517 HTTP
          transient)          ▲ bootstrap: GET page 0 (header+PRIMARY),      | --html fetch())
                              │ Content-Length, and the LAYOUT IDENTITY
                              │
   store_load_*  ──▶ find/range over PagedReader ──▶ deep-copy each match ──▶ local heap Store
                    (the normal index traversal)     (OpCopyRecord relocates    (self-contained
                                                       rec pointers cross-store)  result)
```

- **PagedReader** — read-only backing: sparse LRU page table + a fetch-on-miss `resolve`. Memory =
  resident pages only. Bootstrap eagerly gets page 0 (header + `PRIMARY` + index start) + the total
  size.
- **The @PLN97 gate (bootstrap, mandatory):** before trusting any record, read the remote store's
  **layout identity** and compare it with the running program's via
  `schema_sidecar::classify`/`check`. A mismatch → **reject** (`SchemaMismatch`), never range-read
  raw — a misread over the wire is the worst form of the #477 gap. Carry the identity as a
  `.dschema` sidecar (small extra GET) or a `layout_hash` field in the store directory header.
- **Resolve + copy-out** — run the normal `find`/range against `PagedReader`; each matched entry's
  record graph is **deep-copied into `local`** via the existing cross-store copy (`OpCopyRecord` /
  `copy_from_worker`, which already relocate `rec` pointers across stores). Result: `local` holds
  exactly the working set, relocated and self-contained.

## The deterministic test harness (build FIRST, before HTTP)

A **local Range-capable page provider**: a `resolve(off,len)` that reads a real store file from
disk and records every `(off,len)` fetched. This makes phases 1–4 deterministic (no network) and
makes "bytes fetched ≪ file" a **countable, asserted** fact. HTTP (#517) is swapped in only at
phase 5; the provider interface is identical.

## Phased verifiable steps

Each phase is independently landable, each ends green on **`--interpret`, `--native`, and
`--native-wasm` (wasmtime `--dir`)**, and each acceptance test asserts *result-identical AND
bytes-bounded AND leak-clean* where applicable. "Prove it can fail" = the negative control listed.

### Phase 0 — the read-virtualization probe (P0 above)
- **Build:** a throwaway `resolve`-over-a-file + one `find`.
- **Verify:** found `rec` correct; touched ranges = a few pages (logged). **Decides (b1) vs (b2).**
- **Non-regression measure (the second invariant):** micro-benchmark a store-heavy read loop (many
  `addr`/`find`) with the paged code compiled in vs. out; the normal path must time **within noise**
  of baseline. This is what forces (b2) unless a (b1) branch proves free. If (b1) is taken, this
  benchmark graduates to a standing guard so a later change can't silently re-tax the hot path.
- **Prove-can-fail:** a key NOT in the store → `find` returns absent (not a wrong rec); a
  whole-file scan would touch every page (contrast).

### Phase 1 — heap store-load (whole file, no HTTP)
- **Build:** `store_load(path)` — `read_bytes` → 8-aligned heap arena → `Store{ backing: Heap,
  file: None }` → `fl_rebuild()`. ~15 lines mirroring `Store::open` (`store.rs:394`); the wasm
  no-op (`store.rs:387`, `allocation.rs:2217`) gains a real body. Independently useful: unblocks
  **whole-block wasm load** today.
- **Verify:** load a file written by `bind_path`; every query returns **byte-identical** to the
  same query on the mmap-`open`ed store (a `cross_mode`-style transcript). Runs on all three
  backends (this is the piece wasm lacked).
- **Prove-can-fail:** a truncated/garbage file → clean reject (the @PLN97 identity gate + the
  header signature check), not a panic or a misread.

### Phase 2 — paged read-only backing (local provider)
- **Build:** `PagedReader` (page table + LRU + fetch-on-miss `resolve`) over the **local-file**
  provider; the option-B/(b1|b2) result from P0. Bootstrap: page 0 + size + identity gate.
- **Verify:** a `find` / a full iteration over `PagedReader` returns the SAME records as the
  whole-file load, **and** `pages_fetched · page_size ≪ file_size` (assert the count from the
  provider log). Both backends + wasm.
- **Prove-can-fail:** shrink the page cache to 1 entry → still correct (just more fetches) — proves
  correctness is independent of residency; a corrupted page → detected (per-record size-word sanity
  / the identity gate), not a wild read.

### Phase 3 — `store_load_keys` (point lookups; Hash & Sorted)
- **Build:** `store_load_keys(local, url, keys)` — for each key, `PagedReader.find(key)` → deep-copy
  the record graph into `local` (`OpCopyRecord`). Hash and Sorted key paths.
- **Verify:** for the loaded keys, `local` returns records **byte-identical** to the full store;
  keys **outside** the set are absent (exactly as if `local` were built with only those entries);
  `local` is iterable and **leak-clean** (`LOFT_STORES=warn` / the leak gate); bytes fetched ≪ file.
- **Prove-can-fail:** a query for an unloaded key returns absent (not stale bytes); a key present in
  the remote but not requested is NOT in `local`.

### Phase 4 — `store_load_range` (Sorted / Ordered)
- **Build:** `store_load_range(local, url, lo, hi)` — the range-friendly index walk over
  `PagedReader`, deep-copying every entry in `[lo,hi]`.
- **Verify:** a range query over `local` == the same range over the full store (byte-identical
  vector); bytes ≪ file; leak-clean; all three backends.
- **Prove-can-fail:** `[lo,hi]` with `lo>hi` → empty; a partly-covered range loads exactly the
  covered entries.

### Phase 5 — wire the provider to #517 / the `fetch()` bridge
- **Build:** the `resolve` page provider = a `Range: bytes=<a>-<b>` GET — the #517 client under
  native/wasip2, the JS `fetch()` bridge under `--html`. No new traversal code; only the provider
  swaps.
- **Verify (the issue's acceptance test):** host the `soverijssel` block (1215 tiles / 229,117
  ways) behind a local Range server; for a 40-point route, `store_load_range` then
  `tiles_corridor_ways(local, pts, margin)` returns a `vector<Way>` **identical** to running it
  against the full store, **and** bytes fetched ≪ file. Parity on `--interpret`, `--native`,
  `--native-wasm`.
- **Prove-can-fail:** a Range server that returns 200 (whole file) instead of 206 → the reader must
  still be correct (falls back to whole-file) or reject cleanly; a server serving a block written
  under a **different layout** → rejected by the identity gate, never misread.

## Open questions (each with a recommendation)

1. **Page size / eviction** — fixed 64 KiB LRU vs. record-extent-aware (read the size word, GET
   exactly its span). *Rec: start 64 KiB LRU (simple, deterministic to test); add record-aware GETs
   only if phase-5 range-GET counts prove too high at 0.5 GB.*
2. **Arena bound** — cap `local` or grow as copied. *Rec: grow — copy-out is naturally bounded by
   the working set; no cap needed.*
3. **Directory vs. native index** — rely on the store's own `Sorted` index, or emit a compact
   `(key→offset,len)` header. *Rec: native-index-only (simplest); revisit if GET counts are high.
   If a directory header lands, fold the `layout_hash` (the @PLN97 identity) into it — one GET
   gates the whole read.*
4. **Concurrency** — batch the working-set range GETs (multi-range / parallel) to cut phone
   latency. *Rec: land single-GET-per-page first (phases 1–5), then batch as a phase-6 optimisation
   measured against the acceptance test's latency.*

## Effort & sequencing

**Effort MH.** P0 (a day) gates the architecture. Phase 1 is ~S and independently useful (ship it
first — it unblocks wasm whole-block load). Phases 2–4 are the core (M) over the deterministic
local provider. Phase 5 (S) swaps in #517. The identity gate is a few lines reusing
`schema_sidecar`, added at the phase-1 bootstrap and carried through.

**If promoted to a plan:** file a `loft-lang/plans` issue, move this doc to
`doc/claude/plans/<issue#>-remote-store-loader/README.md`, and lift these phases into its Sub-arcs
table. The verifiable steps above are already the plan's acceptance gates.
