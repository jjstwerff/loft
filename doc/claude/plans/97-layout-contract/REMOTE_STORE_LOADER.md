<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->
# Remote working-set store loader — design (#522)

**Tracker:** [loft-lang/loft#522](https://github.com/loft-lang/loft/issues/522). Built as **arc G
of @PLN97** ([README](README.md)) — the layout contract's hardest consumer *and* its cross-network
validation. Pairs with #517 (HTTP stack) and rides the layout contract ([formal/layout.md](../../formal/layout.md)).

**Status — working-set fetcher COMPLETE for HASH and SORTED, local AND remote (2026-07-07/08).** The
partial store fetcher is built and verified end-to-end: `store_load_key` / `store_load_keys` /
`store_load_key_text` (hash point lookups) and `store_load_range` (sorted range) pull only the pages a
lookup touches — **from a local file OR an `http://` `Range` server** — and relocate the matched
entries into a sound local heap. Done + verified (interpret + native, + wasip2 for the whole-file base;
leak-clean; `store_verify` on every load; each with a prove-can-fail control):
- **Phase 1** `store_load` (whole file) · **Phase 2** `PagedReader` (page/LRU/`resolve`) ·
  **Phase 5** `HttpRangeProvider` + `PageSource` (the REMOTE fetch, `ureq`, feature-gated).
- **Phase 3a** bounded find + flat copy · **Phase 4 + 3b.7** `store_load_range` over Sorted (binary
  search over the reader, build the local vector in key order).
- **Relocating copy — ALL shapes:** 3b.1 safe-refusal · 3b.2 text · 3b.3 vector\<scalar\> · 3b.4
  inline nested structs · **3b.4b vector\<struct\>** (`relocate_ptr_fields` — one recursive walk over
  every pointer kind) · 3b.6 text keys. Only `vector<text>` / `vector<vector>` remain SAFELY REFUSED.
- The instrument: **`store_verify`** (built on `validate_claims`, extended with a Hash arm).

**Remaining (hardening / proof / browser — the core fetch works without them; each safely degrades):**
- **3b.5 layout-identity gate**: `schema_sidecar::check` at bootstrap — needs `store_persist_bind` to
  WRITE a `.dschema` beside the store (it doesn't today) + fetch/compare it on load. `store_verify`
  already catches the structural corruption a wrong-layout fetch would produce; the gate adds a
  clean up-front reject.
- **3b.8 bytes-≪-file at scale**: a large-fixture benchmark. The property holds by construction (a
  point lookup touches O(1) pages) and `LOFT_LOADER_STATS` observes `bytes_fetched` vs file.
- **`--html` `fetch()` bridge** (browser target).

> **Retracted (2026-07-08):** an earlier note here claimed a native-codegen bug — a `#rust` builtin
> with a `reference` arg AND ≥2 integer-literal args mis-binding the reference to an unbound
> `_v_local` under pre-eval. It does **not** reproduce: `store_load_range(local, path, lo, hi)` with
> the body `stores.load_range(&(@local), @path, @lo, @hi)` compiles and runs on `--native` (verified
> by `--native-emit` + a real compile across the literal-int, variable-int, and repeated-`@local`
> P203 paths). The original workaround (bounds as a `vector<integer>`) has been reverted to this
> natural scalar API. The in-flight failure that prompted it was almost certainly a missing
> interpreter handler / stale binary, not codegen.

## Two consumers, one general primitive

This is **not** a routing feature — it is a **general "materialize the working set of a remote
store" primitive**, and two consumers make it worth building well:

- **Data / routing** (filed the issue): a phone routes over a 0.5 GB block by pulling only the
  handful of ~2 km tiles a route touches.
- **Game asset streaming** (the strategic driver): a game — running in the **browser** (wasm) —
  streams only the assets a scene needs (meshes, textures, level chunks, prefabs) from a server,
  keyed by id or region, without shipping the whole asset store. This is the pull/working-set
  complement to @PLN18's push channels (05c bulk broadcast pushes the SAME pack to N seats; this
  pulls each seat's OWN working set), and the natural home of a future asset pipeline.

Build it as the general primitive; both consumers are just `store_load_keys` / `store_load_range`
over a keyed / sorted asset store. **Design consequences of the game target** (first-class, not
afterthoughts): the **browser `--html`/`fetch()` path is a primary target** (games live there);
asset records are **large** (a mesh, a texture), so **record-extent-aware fetches** matter (§ open
questions); and a scene load wants **batched** working-set GETs (§ concurrency). A "good
implementation" means these hold on day one, not bolted on.

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
formalised in [layout.md](../../formal/layout.md): record 0 = header, record 1 = `PRIMARY` root,
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

**How to virtualize the read without taxing the hot path** — the paged store is **read-only**, so
only the read subset (field/element/index reads + the `find`/range traversal) ever runs against it;
mutation ops never do. Three ways to route those reads through `resolve`:

- **(b-branch)** — add a residency branch to the shared `addr`/`OpGetField`. **Rejected:** it taxes
  *every* program's every read (violates the second invariant).
- **(b-dup)** — DUPLICATE the read subset into paged opcodes (`OpGetFieldPaged`, …), dispatched
  only for a paged store. Keeps the hot path clean, but **complicates IR generation**: it adds a
  *second set of read ops* the IR generator must never mix — it would have to know each read site's
  store-backing to emit the right variant, and native codegen needs two emit paths per read.
  Op-budget itself is fine (~225 free of 512, and the ceiling is extensible via the proven
  escape-range technique — 255/254 → a new range), but the IR-mixing risk isn't worth it.
  **Rejected on that ground.**
- **(b-builtin) — a self-contained Rust traversal behind the `store_load_*` builtins.** The paged
  read is Rust code (its own `resolve` + `find`/range over a byte-reader); the loft surface is just
  a builtin *call*, so **IR generation is UNCHANGED — no new opcodes, nothing to mix**, the hot
  path is untouched, and the whole paged path is isolated. Cost: reimplement `find`/range over a
  byte-reader (bounded, isolated Rust). **Recommended.**

**Recommendation: (b-builtin).** It satisfies both invariants — byte-identical working set AND zero
cost to non-users (no IR change, no hot-path branch, feature-gated deps) — and keeps the two op
sets from ever mixing. **P0 *is* this spike** (`resolve` + one `find` in Rust); it de-risks the
whole approach before any phase.

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

### Phase 0 — the read-virtualization probe (P0) — **DONE 2026-07-07**
- **Read-virtualization is sound.** `hash::find` (src/hash.rs:89) is an O(1) bucket lookup over
  `get_u32_raw(rec, fld)` = a read at `rec·8 + fld` — a handful of words across ~2 records (the
  bucket slot + the entry record), independent of hash size. So a byte-offset reader (`resolve`)
  can serve it and it touches **O(1) pages**, not O(file). `b-builtin` confirmed (a Rust traversal
  over a byte-reader needs only these offset reads; no shared-`Store` change → the second invariant
  holds by construction).

**But P0 found a load-bearing BLOCKER the design missed** — see [§ P0 results](#p0-results--the-portability-prerequisite-2026-07-07). The hash INDEX was not portable across
processes, so the loader phases below gained a **prerequisite**: make persisted collections
cross-process-portable first.  **Both halves are now DONE** — 0.5a (#523: hash seed in the bucket
record) and 0.5b (`store_persist_bind` accepts `sorted`/`index`), so `store_load_keys` (Phase 3)
and `store_load_range` (Phase 4) are unblocked.

### Phase 1 — heap store-load (whole file, no HTTP) — **DONE 2026-07-07**
- **Built:** `store_load(r, path)` — the portable, non-durable counterpart of `store_persist_bind`.
  `Store::load` (`store.rs`) reads the file into an 8-aligned heap arena (`file: None`) →
  `fl_rebuild()`; `Stores::load_path` (`allocation.rs`) swaps it into the collection's slot
  (mirrors `bind_path`'s existing-file branch, minus mmap + the fresh/create path); the
  `n_store_load` interpreter handler (`native.rs`, **ungated** — the piece wasm lacked) + the
  `#rust"stores.load_path(…)"` builtin (`02_files.loft`). Unblocks **whole-block wasm load** today.
- **Verified:** a hash written by `store_persist_bind` reloads via `store_load` into a FRESH
  heap-backed hash with the same keys AND a correct key lookup (`h[13]=1300`) — the 0.5a
  bucket-seed makes it portable — on **all three backends**: interpret, native, and **wasip2**
  (`wasmtime`, fixture preopened). Guards: `store_persist_loft.rs::store_load_reads_persisted_image_both_backends`
  + `tests/scripts/store_load_smoke.loft`.
- **Prove-can-fail:** a garbage / non-store file → `store_load` returns `false` (the `Store::load`
  signature check under `catch_unwind`), not a panic or misread —
  `store_persist_loft.rs::store_load_rejects_garbage_file`.
- **Deferred to Phase 5:** the @PLN97 layout-identity gate on load (currently the header
  `SIGNATURE` check only); wire `schema_sidecar::check` at the bootstrap when the remote provider
  lands (a wrong-layout local file is far less likely than a wrong-layout remote fetch).

### Phase 2 — paged read-only backing (local provider) — **reader core DONE 2026-07-07**
- **Built:** `src/paged_reader.rs` (behind the `remote-store` feature — the zero-cost gate): the
  `PageProvider` trait, `LocalFileProvider` (reads ranges from disk, **logs every `(off,len)`** so
  "bytes fetched ≪ file" is countable), and `PagedReader` — a sparse LRU page table + fetch-on-miss
  `resolve(off,len)` that coalesces boundary-spanning reads, plus the typed reads (`u32_at` /
  `i32_at` / `i64_at` / `record_words`) that mirror the `Store` accessors (`byte = rec·8 + fld`,
  native-endian). The option-B / `b-builtin` result from P0.
- **Verified (5 unit tests):** `resolve` returns exact bytes incl. page-span coalescing; past-EOF
  zero-pads; a small read touches **one page**, not the file; correctness is **independent of
  residency** (a `capacity == 1` reader agrees byte-for-byte with a fully-resident one — the
  prove-can-fail); typed reads are native-endian at `rec·8`.
- **Re-slice:** the **`find`/range traversal over `PagedReader` moves to Phase 3/4** — it needs the
  collection's key metadata (`Key[]`) and root, which the `store_load_keys`/`_range` builtins carry
  naturally, so it is testable end-to-end there (the "same records as whole-file, bytes ≪ file"
  gate lands with Phase 3). Phase 2 is the reader **mechanism**; the traversal that rides it is
  Phase 3. Bootstrap (page 0 + size + the @PLN97 identity gate) also lands at the Phase-3 entry.

### Phase 3 — `store_load_keys` (point lookups; Hash & Sorted)

**Phase 3a DONE 2026-07-07 — `store_load_key` + `store_load_keys` (integer keys, flat struct).**
The lowest-risk cut is shipped and verified, singular AND plural:
`store_load_key(local, path, key)` / `store_load_keys(local, path, keys: vector<integer>)` fetch the
requested integer-keyed entries from a persisted hash image into `local`, reading only the pages the
lookups touch (the paged reader is opened once and its cache reused across keys; the plural returns
the count found). `Stores::load_key` (allocation.rs, `remote-store`-gated) opens a `PagedReader`, takes the
root from `local`'s live `DbRef` + the `Key[]` from `stores.keys(known_type)` (the design unlock —
NOT reverse-engineered bytes), runs `paged_reader::find_hash_entry` (a read-only port of
`hash::find`), then FLAT-copies the matched record's scalar fields into a fresh `local` claim and
links it via the verified `hash::add` (no relocation — flat struct has no owned children). Wired as
`n_store_load_key` (native.rs, ungated-by-mmap) + the `store_load_key` builtin (02_files.loft).
**Verified interpret + native + wasip2 (wasmtime), leak-clean:** the requested key loads with the
right value, un-requested keys are absent, `len == 1` (bounded working set) —
`store_persist_loft.rs::store_load_key_loads_only_the_requested_key_both_backends` (+ a `loadkey`
run under wasmtime with the fixture preopened). `LOFT_LOADER_STATS` prints `bytes_fetched` vs file
for the ≪-file check at scale.

**Phase 3b — the relocating graph-copy (detailed, individually-verifiable steps).**
The remaining work extends the copy from FLAT (scalar-only) entries to entries with heap fields
(text / vector / nested), plus more key types and the Sorted path. Each step below is independently
landable, has a concrete pass/fail check, and is gated on both backends + leak-clean.

**The two verification instruments (both wired into every 3b step).**
- **Structure — `store_verify(r)` (built 2026-07-07).** A loft builtin +
  `Stores::verify_graph_ok` that runs the DEFENSIVE `validate_claims` walk (guard-before-deref: it
  NAMES a broken edge instead of faulting on it). Extended with a bounds-checked Hash arm so it
  covers keyed collections. After any `store_load*`, `store_verify(local)` proves the copy left **no
  pointer aimed outside the store** (the exact failure a bad relocation produces — a source
  rec-number larger than the small local store). Positive-control-tested (it must CATCH an
  out-of-range bucket pointer, not crash) and wired into the loader regression
  (`loadkey verify=true`). Covers the "is the store in the right format / structurally sound"
  half of the confidence question.
- **Content — differential against whole-file `store_load`** (the "corresponds to the original"
  half):
Phase 1's `store_load` is verified, so it is the GROUND TRUTH: load the whole persisted collection
into `g_full`, then `store_load_keys(subset)` into `g_partial`, and assert — for every requested
key — `g_partial[k]` deep-equals `g_full[k]` field-by-field (including every heap field's contents),
that un-requested keys are ABSENT from `g_partial`, and `len(g_partial) == |subset|`. This turns
"did the relocating copy corrupt anything?" into a mechanical check with no reverse-engineering —
the safe way to build heap-mutation code. Add it as a helper in `store_persist_loft.rs`
(`assert_subset_matches_full`) driven by a `.loft` script that prints each entry's fields; wire it
into every 3b step's regression.

| Step | Build | Verified by (all on interpret + native, leak-clean) |
|---|---|---|
| **3b.1 field classifier + SAFE REFUSAL** | classify each entry-struct field via the type table (`self.types[field.content].parts` / `Type`) into {inline-scalar · text · vector · nested-struct · other}; `load_one` REFUSES (returns `false`, copies nothing) any entry with a non-scalar field. No copy behaviour change yet. | a `hash<Rec[id, name: text]>` → `store_load_key` returns **false** (clean refusal, NOT a corrupt/partial entry); the flat-int case is unchanged. Proves the classifier is correct BEFORE any risky copy — the load-bearing safety step. |
| **3b.2 text-field relocation** | for a `text` field, flat-copy the source string sub-record (header + len + UTF-8 bytes — itself flat) into a fresh `local` claim, then overwrite the field's `u32` pointer with the new local rec. | differential gate on `hash<Rec[id, name: text]>`: `g_partial[k].name == g_full[k].name` (and a distinctive long string > one page, to exercise a multi-page string). |
| **3b.3 vector<scalar>-field relocation** | copy the vector's inner record + its length-prefixed element bytes into `local`, relocate the field pointer. | differential on `hash<Rec[id, tags: vector<integer>]>`: length AND every element match `g_full`. |
| **3b.4 recursion: nested struct + vector<struct>** | recurse the copy through in-place nested structs and `vector<struct>` elements (mirror `for_each_owned_child`'s Struct/Vector arms). | differential on `hash<Rec[id, sub: Sub{a,b}]>` and `hash<Rec[id, items: vector<Sub>]>`. |
| **3b.5 layout-identity gate at bootstrap** | at load, read the remote store's layout id (`schema_sidecar::classify`/`check`) and REJECT on mismatch before any read. | a fixture written under a DIFFERENT layout → `store_load_key` returns **false** (rejected, never misread); a matching-layout fixture still loads. |
| **3b.6 text keys (key type 6)** | extend `key_compare_reader` to read a `text` key over the reader (string sub-record) + a `vector<text>` key entry point (`store_load_keys_text`). | differential on `hash<Rec[name: text, val: int]>` keyed by `name`. |
| **3b.7 Sorted find path** | `find_sorted_entry` — binary search over the sorted vector via the reader; dispatch on the collection's `Parts` (Hash vs Sorted). | differential on `sorted<Rec[id]>` (bridges into Phase 4 range). |
| **3b.8 bytes ≪ file at scale** | (no new code) a large fixture: N≫1 entries spanning many 64 KiB pages. | `LOFT_LOADER_STATS` asserts `bytes_fetched` for a small subset is a handful of pages, `≪ file` — the working-set invariant made quantitative. |

Order = lowest-risk first: 3b.1 (refuse, no mutation) locks safety; 3b.2–3b.4 add relocation one
heap-kind at a time, each diff-verified; 3b.5 hardens the boundary; 3b.6–3b.7 broaden reach; 3b.8
proves the payoff. A step lands only when its differential gate is green on both backends and leak-clean.

- **Build (full):** `store_load_keys(local, path, keys)` — a `#rust` builtin + `n_store_load_keys` handler
  (mirrors `store_load`'s wiring). The handler:
  1. `PagedReader::open(path)` + the @PLN97 identity gate (read the layout id, `schema_sidecar::check`).
  2. **Root + keys come from the live schema, NOT the bytes** (the design unlock, 2026-07-07): the
     source hash's root is `local`'s own runtime `DbRef` `(rec, pos)` — `local` and the persisted
     image share a collection type, so they share the structural root position — and the `Key[]` are
     `stores.keys(local.known_type)`. This is why `find`/copy MUST live in the builtin (schema in
     hand), and why hand-reverse-engineering the persisted layout is the WRONG (corruption-prone)
     path — a `--interpret` probe confirmed the raw bucket bytes don't self-describe cleanly.
  3. `find`-over-`PagedReader`: port `hash::find` (src/hash.rs:137) to read via
     `PagedReader::u32_at`/`i64_at` instead of `Store::get_u32_raw`; `keys::key_hash(key, seed)` is
     reused verbatim (it hashes the `Content` key, not a store).
  4. **Copy-out (the high-risk core):** reimplement the `for_each_owned_child` cascade
     (allocation.rs:95 — 9 `Parts` variants) over the reader to walk the matched record's owned
     graph, `claim` each record in `local`, copy its bytes, and **relocate** the internal `rec`
     pointers to `local`'s positions; then `hash::add` the entry into `local`. Start FLAT-struct
     (scalar fields = no owned children, no relocation — the lowest-risk cut, and the tile-index
     shape) and extend to vector/text/nested per the cascade. Verify every variant on both backends.
- **Verify:** for the loaded keys, `local` returns records **byte-identical** to the full store;
  keys **outside** the set are absent; `local` is iterable and **leak-clean** (`LOFT_STORES=warn`);
  the `LocalFileProvider` fetch log asserts bytes fetched ≪ file. This is the phase where the
  "find over PagedReader returns the same records" gate (moved from Phase 2) is proven end-to-end.
- **Prove-can-fail:** a query for an unloaded key returns absent (not stale bytes); a key present in
  the remote but not requested is NOT in `local`; a wrong-layout image is rejected by the identity
  gate, not misread.
- **Risk note:** the copy-out mutates `local`'s heap with relocation — the exact class loft's
  stability campaign hardened. It is built variant-by-variant with both-backend + leak verification,
  NOT rushed; a hasty graph-copy here would re-introduce the store-corruption class.

### Phase 4 — `store_load_range` (Sorted / Ordered)
- **Build:** `store_load_range(local, url, lo, hi)` — the range-friendly index walk over
  `PagedReader`, deep-copying every entry in `[lo,hi]`.
- **Verify:** a range query over `local` == the same range over the full store (byte-identical
  vector); bytes ≪ file; leak-clean; all three backends.
- **Prove-can-fail:** `[lo,hi]` with `lo>hi` → empty; a partly-covered range loads exactly the
  covered entries.

### Phase 5 — wire the provider to #517 / the `fetch()` bridge — **HTTP (native) DONE 2026-07-07**
- **Built (native/wasip2 HTTP):** `HttpRangeProvider` (`paged_reader.rs`) — a `PageProvider` that
  GETs `Range: bytes=a-b` via `ureq` (size from a `Content-Range` `0-0` probe + `Content-Length`
  fallback; a `200`/whole-file response falls back to skip-to-offset). A `PageSource` enum
  {`Local`, `Http`} is chosen by the path scheme (`http(s)://` vs a file path), so the generic
  `PagedReader` + the find/relocating-copy stay ONE code path — **no new traversal code, the provider
  just swaps** (exactly as designed). `ureq` moved under the `remote-store` feature (zero-cost gate
  verified: a lean build compiles none of it). `store_load_key(local, "http://…", key)` works.
- **Verified interpret + native:** `store_load_key` over an `http://` URL against a minimal
  `Range`-capable test server loads the key with the right value, bounded (`len == 1`), and
  `store_verify == true` — the same sound working set as the local path
  (`store_persist_loft.rs::store_load_key_over_http_range` + the `serve_ranges` server). The `200`
  prove-can-fail path (server ignores `Range`) is handled (skip-to-offset).
- **Remaining:** the `--html` JS `fetch()` bridge (browser target); the `soverijssel`-block route
  capstone (waits on Phase 4 `store_load_range`); the layout-identity gate at bootstrap (3b.5).

#### original design
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

## P0 results — the portability prerequisite (2026-07-07)

P0 confirmed read-virtualization is sound (above) but surfaced a **load-bearing blocker the design
assumed away** — and one the whole feature rests on: **a persisted hash is not portable across
processes.**

**Empirically confirmed** (`store_persist_bind` a `hash<Rec[id]>`, look up a key, in two separate
processes):

| | build (process A) | reopen (process B) |
|---|---|---|
| iterate (count) | 3 | **3** — the data survives |
| `h[2]` lookup | v=20 | **v=null** — the key is NOT found |

**Root cause:** `keys::key_hash` uses a **per-process random seed** (`RandomState::new()` from
`getrandom`, memoised per-process — the P253 hash-DoS fix, `src/keys.rs:24`). So a hash's bucket
layout is process-specific: a different process (a remote reader — or even a local re-open)
re-hashes the key to a **different bucket** and misses it. The records are intact; only the hash
INDEX is unreadable by anyone but the writer's process.

**Two consequences:**
1. This is a **pre-existing bug in `store_persist_bind`**, independent of #522 — a persisted hash
   cannot be key-looked-up after a restart. (The #513 test only checked *iteration/count*, so it
   missed the lookup failure.) **Filed separately.**
2. `store_persist_bind` today is **hash-only** — it rejects `sorted<…>` (*"expected hash, got
   sorted"*). The `sorted`/range path (comparison-based, **no seed** → portable) does not exist yet.

**Decision — arc G gains a Phase 0.5 prerequisite (before any loader phase):** make persisted
collections cross-process-portable.

### Phase 0.5a — persist the hash seed — **DONE 2026-07-07 (#523)**

The seed is now **per-hash, stored IN the hash's own bucket record** (not per-store in the header
or a sidecar, as this doc first recommended).  `keys::fresh_seed` draws a random 64-bit seed when a
hash is first populated; `hash::add` writes it into the bucket record (word 1, byte 8), carries it
across every rehash, and `find`/`remove`/the probe read it back so `keys::seeded_hasher(seed)` — a
**fixed-key** `DefaultHasher` mixed with the seed — maps a key to the SAME bucket in every process.
A reader (fresh process, remote, or `store_load_keys`) re-derives identical buckets straight from
the persisted bytes, with **no header/sidecar change** — the durable file stays bit-for-bit the
in-memory store (the @PLN97 "one format" law).  P253's DoS defense is preserved: the seed is still
random per hash, so an attacker can't precompute collisions without it.

Why the bucket record rather than the store header / sidecar: it is self-contained (the seed
travels with the buckets it governs, no separate load step), it does not touch the base store
format (sacred payload), and it is a *deliberate* hash-layout change the @PLN97 golden test would
catch and version.  Bucket layout: word 0 = `[room | length]`, **word 1 = seed**, words 2.. =
buckets; `elms = (room - 2) * 2` (initial claim bumped 9→10 to keep 16 slots).  The bucket-walk
now lives in ONE place — `hash.rs`; `for_each_owned_child` routes through `hash::records` (the
free/copy cascade no longer re-encodes the layout).  Pinned by the two-process lookup assertion in
`tests/store_persist_loft.rs::fresh_then_reload_round_trip` (reload process must read `h[13]=1300`,
not null).

### Phase 0.5b — extend persistence to `sorted`/`index` — **DONE 2026-07-07**

`store_persist_bind` now accepts any store-rooted keyed collection, not just `hash`.  The blocker
was purely the loft type signature (`r: hash`): `bind_path` snapshots the whole dedicated Store's
bytes and is collection-agnostic, and `sorted`/`index`/`spacial` are comparison-based (no
per-process seed) so their persisted image is portable by construction.  Since a `#rust`-body
builtin cannot be overloaded (`Cannot redefine`), the fix widened the parameter to a bare
`reference` and taught the type checker that a keyed-collection handle (`Type::Hash` / `Sorted` /
`Index` / `Spacial`) satisfies a bare `reference` param — it is already a `DbRef`, so no conversion
op (`convert` + `can_convert` in `src/parser/mod.rs`).  (`ordered` is not a distinct user-facing
`Type` — only a storage `Parts` variant — so it needs no separate work.)  Pinned by the two-process
`store_persist_loft.rs::sorted_fresh_then_reload_round_trip` (reload process reads `s[13]=1300` and
iterates in key order).

Net: the read mechanism is fine, and the **foundation (portable persisted collections) is now
complete** — the keys half (0.5a, hash) and the range half (0.5b, sorted/index) both persist and
cross-process-read correctly.  `store_load_keys` (Phase 3) and `store_load_range` (Phase 4) are
unblocked.  P0 caught the whole thing before a line of the loader was written.

## Open questions (each with a recommendation)

1. **Page size / eviction** — fixed 64 KiB LRU vs. record-extent-aware (read the size word, GET
   exactly its span). *Rec: 64 KiB LRU for the index traversal (small word reads); but the
   **game-asset** driver tips toward **record-aware GETs for the leaf payload** — a large mesh /
   texture record spans many pages, and one range GET of its exact span beats N page GETs. So:
   64 KiB pages for traversal, record-span GETs for the matched records' bytes.*
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

**Effort MH.** P0 (done) confirmed the read mechanism AND found the portability blocker. **Phase
0.5 (the prerequisite): make persisted collections cross-process-portable** — **0.5a (DONE, #523):
persist the hash seed** (fixed the pre-existing bug + unblocks the keys path); **0.5b (DONE):
`store_persist_bind` accepts `sorted`/`index`** (the portable range path, needed before Phase 4).
Then: Phase 1 is ~S and independently useful (ship it — it unblocks wasm whole-block load);
Phases 2–4 are the core (M) over the deterministic local provider; Phase 5 (S) swaps in #517. The
identity gate is a few lines reusing
`schema_sidecar`, added at the phase-1 bootstrap and carried through.

**As @PLN97 arc G:** this design is a sub-file of the layout-contract plan (adjacent — a remote
range-read is the hardest test that the layout is stable *and* portable). The six phases above are
arc G's build steps; their acceptance tests are its gates. Sequence it **after** the durable-store
consumer wiring (Phase D slice 2's residual) — both need the same `check_beside` gate on open, so
land that once and reuse it here.
