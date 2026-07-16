<!-- SPDX-License-Identifier: LGPL-3.0-or-later -->
# @PLN105 — layout-aware zero-copy FFI delivery (`deliver` / the loft→JS binary bridge)

> **Issue:** [loft-lang/plans#105](https://github.com/loft-lang/plans/issues/105) ·
> **Labels:** `subject:loft` · `status:next` · **Filed 2026-07-13.**
> This file is the living source of truth; the issue is the cross-ecosystem id.
> Additive (`deliver`/`expose` + host-imports) → **post-freeze**, not part of @PLN102.
>
> - **Motivation (consumer):** `../routing/docs/loft-binary-bridge.md` — the base-map `view`
>   serializes ~230k features to text in wasm, which JS re-parses with `parseFloat` over
>   millions of coordinate strings. This is the front-end bottleneck. Games want the same
>   primitive for per-frame vertex/index/uniform buffers.
> - **Not freeze-blocking:** `deliver`/`expose` are new `pub fn`s and the host-imports are
>   *additive* to the `--html` shim — nothing existing changes, so this can land after 1.0.

---

## The one invariant

> JS reads **deep into any loft value in wasm linear memory — record, vector, enum, ref,
> nested to any depth — driven only by a self-describing layout descriptor loft delivers
> alongside the value, with no serialization and no copy**, and *without JS ever knowing the
> index / hash / spatial layout*: keyed collections are never structural, they are walked
> through opaque cursors, so JS only ever interprets `{scalar, text, record, vector, ref, enum}`.

The whole design is one move: **give JS the exact type-driven walk `read_data`
(`src/database/io.rs:51`) already does inside loft.** `read_data` recurses over
`self.types[tp].parts` (the `Parts` enum, `src/database/mod.rs:148`) — `Struct` →
fields, `Vector`/`Array` → elements, `Enum`/`EnumValue`, scalars — and **panics** on
`Sorted`/`Ordered`/`Hash`/`Index`/`Radix`/`DbRef`/`ChildRec` when they appear as a field
type (`io.rs:213-238`). That panic is not a limitation to route around — it *is* the
design boundary: **keyed collections have no byte layout a reader can walk; they are
iterated.** We transcribe the walkable half of `Parts` into a descriptor JS can read, and
mark the un-walkable half `Iterated`.

---

## Design

### 1. The layout descriptor — `Parts`, transcribed for a foreign reader

Emit, **once per `typeId`** (memoized), a compact self-describing node. It is the
read-only twin of the schema `read_data` consumes — same information, external encoding.
Node set (a total function of `Parts`):

| Descriptor node | Source `Parts` | JS does |
|---|---|---|
| `Scalar{kind, offset, width, nullSentinel}` | `Base` (bool/u8/int/long/single/float), `Byte`/`Short`/`ShortRaw`/`Int` | read N bytes at `rec+offset`, compare against sentinel → `null` |
| `Text{offset}` | `Base` (text) | follow the interned-string ref, copy out the UTF-8 bytes (`STRING_NULL` → `null`) |
| `Record{typeId, fields:[{name, offset, typeId}]}` | `Struct(fields)`, `EnumValue(_, fields)` | recurse per field at `rec+offset` |
| `Vector{elemTypeId, byRef}` | `Vector(elem)` (inline), `Array(elem)` (`byRef=true`, holds refs) | read len + elements; **scalar-vector fast lane** below |
| `Ref{targetTypeId}` | `DbRef` | read the 12-byte `DbRef` (`store_nr,rec,pos`); `rec==0`/`store_nr==u16::MAX` → `null`; else recurse |
| `Enum{variants:[{disc, name, payloadTypeId?}]}` | `Enum(values)`, struct-enum | read the discriminant; value-enum → name; struct-enum → recurse into the variant record |
| `Iterated{elemTypeId}` | `Sorted`/`Ordered`/`Hash`/`Index`/`Radix`, `ChildRec` | **never structural** — walk via cursor (§3) |

The descriptor carries exactly the byte facts `read_data` already computes: per-field
offset + width + null sentinel come from the same `typedef.rs` offset table and the same
`Parts` variants. **No new layout knowledge is invented** — if `read_data` can walk it,
the descriptor can describe it; if `read_data` panics on it, it becomes `Iterated`.

### 2. The generic JS reader — one function, blind to keyed layout

```js
// view: DataView over module.exports.memory.buffer (re-derived each deliver — see §5)
// desc: the descriptor table, keyed by typeId
function read(view, desc, typeId, rec, pos) {
  const node = desc[typeId];
  switch (node.kind) {
    case 'scalar': { const v = readScalar(view, node, rec + node.offset);
                     return v === node.nullSentinel ? null : v; }
    case 'text':   return readText(view, rec + node.offset);      // null on STRING_NULL
    case 'record': { const o = {}; for (const f of node.fields)
                       o[f.name] = read(view, desc, f.typeId, rec, f.offset /* +field base */);
                     return o; }
    case 'vector': { const n = readLen(view, rec);
                     if (isScalar(desc[node.elemTypeId]))          // fast lane:
                       return typedArrayView(view, node, rec, n);  //   Int32Array/Float32Array, ZERO copy
                     const a = []; for (let i=0;i<n;i++) a.push(read(view, desc, node.elemTypeId, elemRec(rec,i), 0));
                     return a; }
    case 'ref':    { const {store,r,p} = readDbRef(view, rec + pos);
                     return isNull(store,r) ? null : read(view, desc, node.targetTypeId, r, p); }
    case 'enum':   { const d = readDisc(view, rec); const vr = node.variants[d];
                     return vr.payloadTypeId==null ? vr.name : {tag:vr.name, ...read(...vr.payloadTypeId...)}; }
    case 'iterated': return readIterated(view, desc, node, rec);  // §3 — cursor, not layout
  }
}
```

The **scalar-vector fast lane** is the zero-copy win the consumers need:
`vector<single>` / `vector<int>` → an `Int32Array` / `Float32Array` view straight over
wasm memory, handed to `gl.bufferData(view)` with no intermediate array and no
`parseFloat`. Fixed-point `i32` coordinates survive **exactly** — the current text path
loses precision at 6 decimal places.

### 3. Keyed collections — `Iterated`, walked by cursor

JS must not know B-tree / hash / spatial layout. So a keyed field is delivered as
`Iterated{elemTypeId}` and its elements are pulled through opaque cursors:

```
loft_iter_open(handle) -> cursor      // handle = the Iterated field's location
loft_iter_next(cursor)  -> rec | 0    // 0 = exhausted
loft_iter_close(cursor)
```

Each yielded `rec` is read with the generic reader at `elemTypeId`. JS sees a plain
sequence of records; the index/hash/spatial structure stays entirely inside loft. (Cursors
are the recommended form; a pre-flatten-to-`Vector` variant is the fallback if cursor
state proves awkward under the borrow window.)

### 4. loft-side surface — `deliver` / `expose`

```loft
// deliver a value to the host under a numeric tag; synchronous, borrow ends when deliver returns
pub fn deliver(tag: integer, value: T) fs#deliver;
// expose = deliver a long-lived handle (games' per-frame buffers) — borrow spans frames
pub fn expose(tag: integer, value: T) fs#deliver;
```

Lowering (additive to the current 5-import `loft_io` shim —
`loft_host_print`/`input_len`/`input_copy`/`http_get`/`http_get_copy`, see
`doc/loft-gl-wasm.js:116` `buildLoftImports`):

```
loft_host_deliver(tag, store_nr, rec, pos, typeId)   // the root handle
loft_layout_desc(typeId) -> ptr                       // descriptor blob for a typeId (memoized host-side)
loft_iter_open/next/close                             // §3
```

### 5. The borrow contract (the load-bearing safety invariant)

- Delivery is **synchronous inside one `loft_host_deliver` call**. For `deliver`, the
  borrow ends when it returns; JS must finish reading (or copy out) before then.
- loft **must not grow / realloc / free** the delivered value during the borrow.
- JS **re-derives every view from `memory.buffer` on each entry** — `memory.grow()`
  **detaches** the old `ArrayBuffer`, so a cached `DataView` is a use-after-detach.
- `expose` (long-lived) additionally pins the value against free until a matching release;
  its borrow window spans frames, so it needs an explicit lifetime handle.

This mirrors loft's own deps/borrow model (`OWNERSHIP_MODEL.md`): the delivered handle is
a **View**, not an owned transfer; the descriptor + memory are the backing store.

### Relationship to @PLN97

The descriptor is the **runtime-queryable form of the @PLN97 layout contract** (F9
layout-hash). @PLN97 pins the layout so it can't drift; this delivers that same layout to a
foreign reader. The F9 layout-hash is the natural integrity check on the descriptor
(§ Tests, Phase 0).

---

## Implementation — phased, falsifier-first

Each phase lands behind the parity gate and states the probe that could prove it wrong
*before* the code. Loft-side first (no FFI), then the boundary, then JS, then the consumer.

### Phase 0 — descriptor emitter (read-only twin of `read_data`), pure loft-side — ✅ DONE

Emit the descriptor from `Parts`; **no FFI yet**. The falsifier is a round-trip entirely
inside loft: a generic reader driven *by the descriptor* must reproduce byte-for-byte what
`read_data` produces directly.

- **Code points:** `src/database/io.rs` (`read_data:51` — the twin to mirror);
  `src/database/mod.rs:148` (`Parts` + `Field`); `src/typedef.rs` (field offsets — the
  descriptor reads the same offset table).
- **Falsifier / test:** build a nested value (`Record{ v: vector<Record>, e: Enum, r: Ref }`),
  emit its descriptor, drive a loft-side reader off it, assert `== read_data` bytes. On
  `--interpret` (no FFI). Positive control: a `hash<T>`/`index<T>` field must emit
  `Iterated` (assert it does **not** panic the way `read_data:213` does on the same input).
- **Integrity:** descriptor hash == the @PLN97 F9 layout-hash for the same typeId.

**Landed** (`src/database/descriptor.rs`, `tests/layout_descriptor.rs`): `LayoutNode`/`LayoutDesc`
+ `Stores::layout_descriptor` (exhaustive `Parts` transcription over the shared `layout_closure`)
+ `Stores::read_via_descriptor` (the descriptor-driven byte reader). Three independent oracles pass
on `--interpret`: **faithfulness** — `LayoutDesc::render_dump` reproduces `Stores::layout_dump`
byte-for-byte, so `layout_hash() == layout_algo_hash` (the F9 integrity check falls out for free);
**sufficiency** — three-way `hand-computed truth == read_data == read_via_descriptor` on a nested
`Record{text, inline Record{integer,single}, vector<integer>, boolean}`, plus an anti-vacuity cell
(a corrupted descriptor DIVERGES); **boundary** — a `hash<T>` field emits `Iterated::Hash` and the
reader *refuses* it (`Err`, not panic). Purely additive Rust (no codegen/stdlib), so both-backend
parity is untouched and the @PLN97 golden is unperturbed.

### Phase 1 — `deliver` stdlib + lowering + loopback host — ✅ DONE (interp+native; `expose`/wasm → Phase 2)

Wire the boundary with a **loopback host** in the test harness (no browser). `deliver`
hands `(tag, store_nr, rec, pos, typeId)`; the harness host reads the descriptor + memory
and reconstructs the value.

- **Code points:** `default/02_files.loft` (the `pub fn deliver/expose` decls, next to
  `store_load_url` at `:433` — same `fs#…` host-fn precedent); `src/native.rs` (interpreter
  `n_deliver` — reconstruct/loopback); `src/generation/mod.rs:1312` (`#[link(wasm_import_module="loft_io")]`
  extern block — add `loft_host_deliver`/`loft_layout_desc`); `src/main.rs:6190-6213`
  (`--html` import allow-list + asyncify list — `deliver` is synchronous so it does **not**
  join the asyncify set, unlike `loft_host_http_get`).
- **Falsifier / test:** loopback deliver of the Phase-0 value on **both backends**; assert
  reconstructed value == original. **Parity gate: interpret == native.**

**Landed** (`src/ffi_deliver.rs`, `default/02_files.loft` `OpDeliver`, `src/parser/control.rs`
`dispatch_call`, `tests/deliver_parity.rs`, `tests/common/cross_mode.rs`). `deliver(tag, value)`
lowers (in `dispatch_call`) to `OpDeliver(tag, value, db_tp)` with `db_tp` filled from the value's
static type; the op is a single `#rust` body `stores.deliver_reconstruct(@tag, @val, @db_tp)` that
feeds **both** backends (fill.rs interpreter handler + native codegen from the one template — the
`native_rs_functions_up_to_date` and `fill_rs_up_to_date` guards stay green). The loopback host
reconstructs the value from its layout descriptor (Phase 0's `read_via_descriptor`) and prints a
deterministic line. **Key design move:** the value is passed BY VALUE (its code, not an
`OpCreateStack` reference), so `@val` is the record `DbRef` on both backends — matching native's
`file_to_bytes(self)` convention and sidestepping the interp/native slot-vs-record deref asymmetry
(@PLN85 p9); one body, no divergence. **Parity gate green:** `tests/deliver_parity.rs` runs a flat
struct and a nested `Record{text, inline Record, vector<integer>, boolean}` under both backends and
asserts byte-identical stdout AND the exact expected bytes (non-vacuous). This is the "read deep into
record + vector without knowing the layout" capability, end-to-end.

- **Deferred to Phase 2** (not blocking): `expose` (long-lived pinning has meaning only with the
  real host / borrow window across frames — a fake loopback alias would be dishonest surface); the
  wasm/`--html` host-imports + asyncify wiring (they need the JS host, which is Phase 2 — wiring an
  import with no host would break `--native-wasm`/`--html`); top-level bare-scalar / bare-vector
  delivery (structs, which recurse into vectors/records/scalars, are the meaty case and are done).
  The descriptor-in-loopback IS descriptor-driven here (uses `read_via_descriptor`), so it is
  already a faithful JS stand-in.

### Phase 2 — the generic JS reader (IN PROGRESS on `tuxedo-pln105-phase2`)

Implement `read(view, desc, typeId, rec, pos)` (§2) in `doc/loft-gl-wasm.js`; export a real
program with `--html`; run it headless under node with a JS deliver host.

**Falsifier-first step sequence** (each verifiable; the whole-slice falsifier is the node
harness at the end):
- **P2.a — descriptor → JSON (the JS contract). ✅ DONE.** `LayoutDesc::to_json()`
  (`src/database/descriptor.rs`) serializes the descriptor to `{nodes:{<id>:node},names,sizes}`
  — the read-only twin the JS `read()` switch dispatches on. JSON (not a binary format) because
  the descriptor is metadata emitted once per type-closure + memoized host-side, NOT the hot
  path (the value bytes are the zero-copy fast lane); hand-rendered (no serde) so it compiles
  into the lean wasm build. Node `kind` tags mirror the `read_via_descriptor` arms. Guard:
  `tests/layout_descriptor.rs::descriptor_to_json_is_well_formed_and_faithful` (balanced +
  every §2 node kind present on the corpus). NEXT contract detail to validate against the JS
  reader: value-enum `disc` ordering (currently the `Choices` vec order).
- **P2.b — wasm host-import + the deliver handle. ✅ DONE.** `loft_host_deliver(tag, base,
  type_id, desc_ptr, desc_len)` declared in the RUNTIME `loft_io` import block (`src/lib.rs`,
  cfg `all(wasm32, not(wasi), not(feature="wasm"))`, next to `loft_host_http_get`) — deliver's
  `#rust` body is runtime code (`ffi_deliver.rs`), so the import lives in the runtime, not the
  generated-program block. `deliver_reconstruct` now cfg-splits: the browser target serializes
  the descriptor (`to_json`), computes the value record's RAW linear-memory address (`base =
  &store.addr::<u8>(rec, pos)` — `Store.ptr` IS a wasm memory address), and calls the import;
  native/interp keep the Phase-1 loopback (`deliver_loopback`, the parity oracle — untouched,
  `deliver_parity` still 2/2). SYNCHRONOUS → NOT added to the asyncify set. JS stubs in both host
  shims (inline `main.rs` + `doc/loft-gl-wasm.js`) route to a `globalThis.loftDeliver` hook so
  the bundle instantiates + a harness observes deliveries. **DESIGN CHANGE from the sketch: the
  descriptor JSON is passed INLINE with the deliver call (`desc_ptr/desc_len`), NOT via a
  separate `loft_layout_desc(typeId)->ptr` export** — the deliver `#rust` body already holds the
  `Stores`, so it serializes the descriptor right there; JS memoizes by `type_id`. This avoids a
  wasm export needing to reach global runtime state. **Verified:** wasm32 lib build clean; `loft
  --html deliver_prog.loft` produces a bundle (module-import check passes; `loft_host_deliver`
  wired in). **Runtime-correctness deferred to P2.d** (the node harness validates the `base`
  address + field-offset reads); TEXT is store-INTERNED (`get_str(get_u32_raw(rec,pos))`), so the
  JS reader (P2.c) needs a string-resolution host-fn, not an inline byte read.
- **P2.c — the generic descriptor-driven JS reader. ✅ DONE.** `readLoftValue(mem, storeBase,
  desc, typeId, rec, pos)` in `doc/loft-deliver.js` — the twin of `read_via_descriptor`, kept in
  lockstep. Addressing `storeBase + rec*8 + pos` (`Store::checked_offset`); handles the whole
  serializable subset: scalar (i64/f32/f64/bool/char) + narrow int (byte/short/int) + INTERNED
  text (resolved inline at `id*8+8` — NO host-fn needed, the P2.b worry was unfounded) + nested
  record/enumvalue + value enum + vector (with the scalar-vector typed-array FAST LANE —
  `Int32Array`/`Float32Array`/`BigInt64Array` zero-copy views) + by-ref array; refuses
  store-internal ref/childrec/iterated exactly as `read_via_descriptor` (cursor-walked in Phase
  3). The P2.b handle was corrected to pass `(store_base, rec, pos)` — a pre-computed root
  address alone can't FOLLOW child records. Both shims updated to the 7-arg handle + expose it
  via `globalThis.loftDeliver`.
- **P2.d — the whole-slice falsifier. ✅ DONE (core).** `tools/deliver_repro.mjs` (node:
  instantiate the `--html` wasm, wire `loft_host_deliver`→`readLoftValue`, capture) +
  `tests/deliver_wasm.rs` (build `--html` → extract wasm → run node → assert). Green:
  `Outer{ name:"hi", inner:{a:7,b:1.5}, nums:[10,20,30], ok:true }` reconstructs BYTE-IDENTICALLY
  to the interpreter loopback (`6869`="hi", `07..`=7, `f83f`=1.5, `0a/14/1e`=10/20/30, `01`=true)
  — **the parity gate interpret == native == --html now holds end-to-end.** REMAINING (small): a
  memory.grow-mid-read safety cell; more corpus shapes (value-enum `disc`, by-ref array, narrow
  ints); inlining the reader into the shims so a real page auto-reconstructs (today they expose
  the handle to `globalThis.loftDeliver`).

- **Code points:** `doc/loft-gl-wasm.js:47` (`buildLoftImports` — add the deliver/desc/iter
  host fns); `src/main.rs` `--html` assembly (embeds this glue).
- **Falsifier / test:** headless node harness loads the exported wasm, supplies the deliver
  host, runs `read(...)`, asserts reconstructed JSON == the interpreter's serialization of
  the same value. **Parity gate: interpret == native == --html, byte-identical.**
  memory.grow-safety cell: force a `memory.grow()` mid-read and assert the reader
  re-derives its view (no detached-buffer throw).

### Phase 3 — keyed collections (design DECIDED 2026-07-15: pre-flatten, NOT JS cursors)

Proves JS reconstructs a keyed collection *without any layout knowledge*.

**Architecture decision — PRE-FLATTEN over JS cursors.** The plan sketched JS calling
`loft_iter_open/next/close` back into wasm. But `loft_start` creates the `Stores` as a LOCAL
`UnsafeCell` on its stack (`let cell = UnsafeCell::new(Stores::new()); … n_main(&cell)`;
`generation/mod.rs:1887`) — there is **no global `Stores`**, so a JS-called `loft_iter_*` EXPORT
cannot reach the running store without making the store global (a large, aliasing-risky change to
loft's explicit `&cell` threading). The plan's own stated fallback — "pre-flatten-to-`Vector` if
cursor state proves awkward under the borrow window" — is therefore the right path, and it
**reuses the P2.c reader unchanged** (a flattened keyed collection is just an array of records).

**Reuse found.** loft ALREADY materialises a keyed collection to an iterable rec-nr vector — the
exact path `for x in hash` uses: `Stores::build_hash_sorted_vec` / `build_hash_unsorted_vec` /
`build_radix_sorted_vec` / `build_radix_range_vec` (`database/allocation.rs:1004+`, all `&mut
self`, returning a `DbRef`). `deliver` runs as `fn deliver(s: &mut State)` (`fill.rs:2319`), so
`&mut Stores` is reachable — `deliver_reconstruct` can take `&mut self`.

**Build plan / subtleties to handle:**
1. **Scratch layout indirection.** `build_rec_scratch` returns a HEADER record (offset-4 → data
   rec; data rec offset-4 → count `n`; offset-8 → `n` u32 rec-nrs). That is `Array`-shaped (by-ref
   records) but with ONE extra header hop the P2.c `array` node does not model — either add a
   descriptor node (`FlatIterated`) that encodes the hop, or normalise the scratch to a plain
   `Array` before delivery.
2. **Per-kind coverage.** hash + radix have `build_*_vec`; **`index`/`sorted` still need a
   materialiser** (or a shared "records in key order" over `Ordered`).
3. **Nested keyed fields.** A struct with a keyed FIELD needs a flattened TWIN of the whole value
   (recursively replace `Iterated` fields with materialised arrays), not just a top-level swap.

**First slice — top-level `hash<T>`. ✅ DONE.** `deliver_reconstruct` is now `&mut self` and, on
the browser target, `deliver_browser` detects the root is `Iterated::Hash`, calls
`build_hash_sorted_vec` (the `for x in h` path, key-sorted), and delivers the scratch `DbRef`
(`{rec: header, pos: 4}`, which points straight at the data record — no extra hop) with a
SYNTHETIC `Array(elem)` descriptor (root id `u16::MAX`, the "no type" sentinel, over the element
closure). The scratch layout lines up EXACTLY with the P2.c `array` node, so the JS reader needed
NO change. Verified end-to-end (`tests/deliver_wasm.rs::deliver_flattens_top_level_hash_in_js`):
inserting `{30,10,20}` reconstructs KEY-SORTED `[{ik:10,name:"ten"},{ik:20,…},{ik:30,…}]` — the
same order as the interpreter's `for x in h` (both use `build_hash_sorted_vec`); JS touches no
hash-layout constant. Loopback (interp/native) still refuses a keyed root with `error=store-
internal` (Phase-1, unchanged) — flattening is browser-only.

**Nested keyed FIELD (a struct with a `hash` field). ✅ DONE.** No flattened-twin copy needed: the
struct is delivered IN PLACE, and each direct keyed field's descriptor node is replaced by a
BROWSER-SYNTHETIC `LayoutNode::FlatArray { elem, data }` carrying the materialised array's fixed
data record (`flatten_record_keyed_fields`; the hash field slot `(val.rec, val.pos + field.pos)`
IS the hash's `DbRef`, since `hash::records` reads the bucket claim there). The struct's own
bytes are untouched — scalar/text fields read in place, the keyed field's in-place hash bytes are
ignored in favour of the FlatArray's fixed record. Top-level and nested now share the `FlatArray`
mechanism. `FlatArray` never appears in a real type descriptor (loopback/@PLN97-hash paths add an
error/placeholder arm), only injected at deliver time. Verified end-to-end
(`deliver_flattens_nested_hash_field_in_js`): `Bag{ label, items: hash<Item>, count }` →
`{label:"mybag", items:[{ik:10,…},{ik:20,…}], count:3}` (scalars in place, hash key-sorted).

**All keyed kinds with a `build_*_vec` materialiser — DONE.** `materialize_keyed` dispatches on the
`Iterated` kind: **`hash`** → `build_hash_sorted_vec` (key-sorted), **`radix`/`spatial`** →
`build_radix_sorted_vec` (Morton/natural order — verified `deliver_flattens_top_level_spatial_in_js`
reconstructs `(0,0),(1,2),(3,3)` == the interpreter's `for m in xs`). Both top-level and struct-field
paths use it. `sorted`/`ordered`/`index` return `None` (delivered nothing / stay Iterated) — no
regression, just not yet supported.

**`sorted<T>` — DONE (reclassify to `Vector`, no materialisation).** VERIFIED empirically: a sorted
collection is stored as an INLINE vector (elements at `sorted_rec[8 + size*i]`, `len` at
`sorted_rec[4]`) kept in KEY ORDER — `sorted_finish` binary-search-inserts each element on every
`+=`, so it is sorted at deliver time. So `keyed_replacement` reclassifies `Iterated::Sorted` →
`Vector(elem)` (the field holds the vector directly, read in place — no scratch, no FlatArray).
[First-guess `Array` (by-ref rec-nrs) gave garbage-but-right-count → proved INLINE, so `Vector`.]
Verified `deliver_reads_top_level_sorted_in_js` + `deliver_reads_nested_sorted_field_in_js`
(inserted `{30,10,20}` → key-sorted `[10,20,30]` == the interpreter's `for it in s`).

**`index<T>` — DONE (new `build_index_sorted_vec` + `#`-field skip).** Added
`Stores::build_index_sorted_vec` (`allocation.rs`): an in-order red-black-tree walk (`tree::first`
→ `tree::next`, mirroring `tree::count`) keyed off the node type's left-child BYTE position
(`8 + fields[left_field].position`), collecting rec-nrs → `build_rec_scratch` → the same `FlatArray`
path. `keyed_replacement` routes `Iterated::Index` to it. One layout subtlety, caught empirically:
an index node is the user record AUGMENTED with `#left_1`/`#right_1`/`#color_1` tree fields, and the
descriptor's `elem` IS that augmented node — so a first pass LEAKED them. Fix: the record reader
(both `read_via_descriptor` AND the JS twin) now SKIPS `#`-prefixed synthetic fields (general — they
are never user data), matching what `for x in ix` sees. Verified
`deliver_flattens_top_level_index_in_js` + `deliver_flattens_nested_index_field_in_js` (insert
`{30,10,20}` → key-sorted `[10,20,30]`, no tree fields). **ALL FOUR keyed kinds
(hash/radix/sorted/index) now deliver.** (`ordered` is an internal kind, not a user top-level
collection.)

**Deep nesting through inline record fields — DONE.** `flatten_record_keyed_fields` was generalised
to a RECURSIVE `flatten_at(desc, node_id, at, synth)`: it descends every INLINE record field (the
root, a direct field, or a field of a nested sub-struct) and replaces each keyed collection at its
single-instance location `(at.rec, at.pos + Σ field.pos)` with its array-shaped node, re-inserting
each changed record/keyed node under a fresh synthetic id (so the shared TYPE descriptor is not
mutated and each location gets its own data record). Verified
`deliver_flattens_keyed_in_substruct_in_js` (`Outer.inner.items: hash` → the hash flattens two
levels down; scalars at both levels stay in place). Arbitrary depth (inline records only).

**Multi-instance (keyed collection behind a VECTOR/ARRAY element) — DONE (the REDIRECT rewrite).**
The fixed-data `FlatArray` couldn't express per-element data (a type node is shared by every element
of a `vector<Bag>`). Reworked to a REDIRECT: `FlatArray { elem }` carries NO data; instead a per-
value `flat` map keyed by the collection's `(rec, pos)` gives each INSTANCE its materialised data
record, serialised alongside the descriptor (`to_delivery_json`, `"<rec>_<pos>": data`). Deliver now:
`collect_keyed` walks the WHOLE value — records AND vector/array ELEMENTS (mirroring
`read_via_descriptor`'s traversal so the `(rec,pos)` match the reader) — materialising every
hash/radix/index instance into `flat`; `rewrite_iterated` turns the type-shared `Iterated` nodes into
`FlatArray` (redirect-read) / `Vector` (sorted, in-place); the JS reader looks each `FlatArray`'s
data up in `flat` by the current `(rec,pos)`. One shared node serves every element. This also
SIMPLIFIED the code (no per-location descriptor clones / synth ids). Verified
`deliver_flattens_keyed_in_vector_elements_in_js` (`vector<Bag>`, each `Bag` its own hash → each
element gets its own materialised array) + all 9 prior tests still green.

**`expose` / `release` — DONE (wiring + pinning; full cross-frame needs the yield harness).**
`expose(tag, value)` is a LONG-LIVED `deliver`: new `OpExpose`/`OpRelease` ops (`02_files.loft`,
lowered in `dispatch_call` like `deliver`), `Stores::expose_value`/`release_value` — expose
materialises the keyed collections + serialises the handle exactly like `deliver`, then PINS the
value's store with a read-only lock (AFTER materialising — a claim on a locked store panics) so its
wasm addresses stay stable across frames; `release(tag, value)` `unlock_store`s it (via the value's
`DbRef`, so no tag→store table). New host imports `loft_host_expose` / `loft_host_release`; both
shims + the harness stash the handle by `tag` (`globalThis.loftExposed`) and drop it on release.
Verified `deliver_expose_and_release_a_value_in_js` (a hash exposed → materialised value read in the
expose call, then `RELEASE`) + the interpreter run proves lock→…→unlock does not crash.
CAVEAT: the single-shot harness reads DURING the expose call (`Stores` is a `loft_start` LOCAL, gone
once it returns) — the true cross-frame read (a game loop yielding to JS between frames, re-reading
`globalThis.loftExposed`) needs the asyncify yield harness, deferred.

**Remaining:** the Phase-2 tails (a memory.grow-mid-read safety cell; inlining the reader into the
production shims); the full cross-frame `expose` test (yield harness).

- **Falsifier / test:** deliver a value containing `hash<T>` + `index<T>` + spatial; JS reads the
  materialised arrays via the existing reader; assert the reconstructed multiset (count + values)
  == the interpreter's iteration. JS touches **no** hash/tree/spatial constant — it only ever sees
  an array of element records.

### Phase 4 — routing migration (consumer acceptance; owned by routing's agent)

Swap routing's `view`/`match` text-serialize for `deliver(...)`; swap the canvas
`parseFloat`-per-coord renderer for the generic reader + typed-array lanes.

- **Code points (read-only, ../routing):** `client/web_basemap_kernel.loft`,
  `lib/map_kernel` (`do_view`/`do_view_bbox`/`do_match`), `browser/store-kernel.mjs`,
  `docs/loft-binary-bridge.md`.
- **Acceptance / test:** routing's existing byte-identical-to-native `view`/`match` check —
  the deliver path must produce the **same** coordinates as the text path, with exact `i32`
  fixed-point (no 6-dp float loss). Perf: no per-frame allocation on the games path.

---

## Tests (the definition of done)

1. **Descriptor round-trip** (Phase 0) — descriptor-driven reader == `read_data`, `--interpret`.
2. **Iterated positive control** (Phase 0) — keyed field → `Iterated`, no structural panic.
3. **Boundary loopback parity** (Phase 1) — deliver reconstruct == original, **interpret == native**.
4. **Headless browser parity** (Phase 2) — **interpret == native == --html**, byte-identical JSON.
5. **memory.grow safety** (Phase 2) — grow mid-read, reader re-derives view, no detach throw.
6. **Cursor reconstruction** (Phase 3) — keyed multiset == interpreter iteration; JS layout-blind.
7. **Routing acceptance** (Phase 4) — deliver `view`/`match` == native text path, exact fixed-point.

The master invariant is the loft-ship parity bar (`loft-ship` skill): a target is done only
when its result *equals the interpreter's* — not merely exits 0.

---

## Relevant code points (consolidated)

| Concern | File:sym |
|---|---|
| The walk to mirror (descriptor twin) | `src/database/io.rs:51` `read_data` |
| The schema it walks | `src/database/mod.rs:148` `Parts` (+ `Field`) |
| Keyed-collection panic = the `Iterated` boundary | `src/database/io.rs:213-238` |
| Field byte offsets | `src/typedef.rs` (offset table) |
| Null DbRef sentinel (`rec==0` / `store_nr==u16::MAX`) | `src/keys.rs` `DbRef::NULL` |
| Host-fn stdlib precedent | `default/02_files.loft:433` `store_load_url` |
| Interpreter native registry | `src/native.rs` (`n_deliver`) |
| wasm extern-import block | `src/generation/mod.rs:1312` |
| `--html` import allow-list + asyncify set | `src/main.rs:6190-6213` |
| Browser host shim | `doc/loft-gl-wasm.js:47` `buildLoftImports` (currently 5 `loft_io` imports) |
| Layout-hash integrity | @PLN97 F9 layout-hash |
| Consumer motivation + acceptance | `../routing/docs/loft-binary-bridge.md` |

---

## Trade-offs & non-goals

- **Only pays for large payloads.** Text stays the default control channel (routing's
  `store`/`view`/`match` command protocol is fine as text); `deliver` is for the
  bulk data behind a command, not for replacing the protocol.
- **`Iterated`, never structural.** We deliberately do **not** expose B-tree / hash /
  spatial layout to JS — that would couple a foreign reader to loft's internal index
  formats and break the moment @PLN48/@PLN97 evolve them. Cursors keep the coupling at the
  element-type surface only.
- **Borrow discipline is on the caller.** `deliver` is a synchronous view; a consumer that
  stashes a `DataView` past the call, or across a `memory.grow`, has a use-after-detach.
  `expose` exists precisely for the frames-spanning case and carries an explicit lifetime.
- **Additive.** No existing behavior changes; this is why it is post-freeze, not part of
  @PLN102.
