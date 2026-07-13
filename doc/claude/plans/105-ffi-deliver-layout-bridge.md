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

### Phase 0 — descriptor emitter (read-only twin of `read_data`), pure loft-side

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

### Phase 1 — `deliver` / `expose` stdlib + lowering + host imports

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

### Phase 2 — the generic JS reader

Implement `read(view, desc, typeId, rec, pos)` (§2) in `doc/loft-gl-wasm.js`; export a real
program with `--html`; run it headless under node with a JS deliver host.

- **Code points:** `doc/loft-gl-wasm.js:47` (`buildLoftImports` — add the deliver/desc/iter
  host fns); `src/main.rs` `--html` assembly (embeds this glue).
- **Falsifier / test:** headless node harness loads the exported wasm, supplies the deliver
  host, runs `read(...)`, asserts reconstructed JSON == the interpreter's serialization of
  the same value. **Parity gate: interpret == native == --html, byte-identical.**
  memory.grow-safety cell: force a `memory.grow()` mid-read and assert the reader
  re-derives its view (no detached-buffer throw).

### Phase 3 — iterator cursors for keyed collections

`loft_iter_open/next/close` + `readIterated`. Proves JS reconstructs a keyed collection
*without any layout knowledge*.

- **Code points:** `src/database/mod.rs` (existing iteration over `Hash`/`Index`/`Radix` —
  cursors wrap it); the `loft_io` extern block + `buildLoftImports`.
- **Falsifier / test:** deliver a value containing `hash<T>` + `index<T>` + spatial; JS
  walks via cursors; assert the reconstructed multiset (count + values) == the
  interpreter's iteration. JS code path touches **no** hash/tree/spatial constant.

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
